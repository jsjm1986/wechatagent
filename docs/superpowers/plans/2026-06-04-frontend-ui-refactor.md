# 前端 UI 重构实现计划（架构 + 视觉）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 13165 行 `App.tsx` + 7488 行全局 `styles.css` 增量重构为 features 切片 + `components/ui` 原子库 + CSS Modules + 4 个 Zustand store + channels 注册表的模块化结构，并落地"易读优先 + 功能元素呼吸 + 6 色语义 token"的视觉语言。

**Architecture:** 增量迁移，旧 `App.tsx` 全程可跑。先建脚手架与 token，再抽原子库，再建 store + Shell 外壳跑通 1 个 channel，然后按 feature 逐个搬（每个一个提交），最后收尾瘦身。每步过 `npm run build` + `npm test` + 浏览器主路径验证。

**Tech Stack:** React 19.2 / TypeScript 5.9（strict）/ Vite 7.2（原生支持 `.module.css`，无需额外配置）/ Zustand 5.0.14（MIT，零依赖，审计已过）/ lucide-react / Vitest 4.1 + @testing-library/react（fetch-mock 风格）。

**关键约束（来自 spec 与项目红线）：**
- 组件样式**禁止十六进制硬编码色值**，必须 `var(--color-*)`。
- 不上 React Router、不引 UI 组件库；保留 state 导航。
- `frontend/src/` 内**禁止** `human-takeover/人工接管/接管/人工` 等字样（`scripts/check-no-human-takeover` 已对前端生效），状态用 AI 自治内部名。
- 不改后端 API 契约。
- 命令在 `frontend/` 目录下运行；项目根含非 ASCII（`工作项目`），用绝对路径或先 `cd frontend`。

---

## 文件结构

| 路径 | 职责 |
| --- | --- |
| `src/components/ui/tokens.css` | 唯一全局 CSS 变量来源（6 色 token + 文字阶梯 + 表面 + 圆角 + 呼吸节奏） |
| `src/components/ui/reset.css` | 极小全局 reset |
| `src/components/ui/StatusBadge/` | 状态标签原子（5 语义色 + 运行态呼吸） |
| `src/components/ui/MetricCard/` `Avatar/` `StatusLine/` `PlanStep/` `EmptyState/` `StageStepper/` `IntentMeter/` | 其余共享原子 |
| `src/stores/navigationStore.ts` | `activeChannel` + `setChannel` |
| `src/stores/accountStore.ts` | `accounts` `selectedAccountId`(localStorage) + 派生 |
| `src/stores/contactStore.ts` | `contacts` `selected` `contactTab` + 派生 `managedCount`/`normalCount` |
| `src/stores/uiStore.ts` | `busy` `error` `llmUsage` |
| `src/app/channels.ts` | `Channel` 类型 + `CHANNELS` 注册表（含 lazy Component） |
| `src/app/Shell.tsx` + `Shell.module.css` | 侧栏 + 顶栏 + 懒加载内容槽 |
| `src/features/<domain>/` | 各业务域：`index.tsx` 入口 + 组件 + `*.module.css` |
| `src/types/index.ts` | 跨 feature 共享类型（从 App.tsx 抽出） |
| `src/lib/api.ts` `lib/format.ts` | fetch 封装 + 格式化 |
| `src/App.tsx` | 收尾后瘦身到 ~80 行：`<GlobalErrorBanner/>` + `<Shell/>` |

迁移期间 `src/App.tsx` 与新结构并存；每搬完一个 feature 删 App.tsx 对应代码。

---

## 阶段 0 — 脚手架与 token

### Task 0.1：建 tokens.css（语义变量唯一来源）

**Files:**
- Create: `frontend/src/components/ui/tokens.css`

- [ ] **Step 1: 写 tokens.css**

```css
/* 全站唯一全局变量来源。组件样式只允许引用这里的 var()，禁止硬编码色值。 */
:root {
  /* —— 语义颜色 token（spec §2.3 的 6 色）—— */
  --color-running:   #30D158; /* 进行中 / 成功（唯一呼吸） */
  --color-scheduled: #0A84FF; /* 已排程 / 关键数据 / 可点击 */
  --color-held:      #FF9F0A; /* 暂缓 / 待核验 */
  --color-blocked:   #FF453A; /* 拦截 / 失败 / 风险 */
  --color-brand:     #5E5CE6; /* AI 身份 / 品牌（不表达状态） */
  --color-inactive:  #8E8E93; /* 未托管 / 停用 / 历史 */

  /* 各语义色的浅色填充（标签底，半透明） */
  --fill-running:   rgba(48,209,88,.14);
  --fill-scheduled: rgba(10,132,255,.13);
  --fill-held:      rgba(255,159,10,.16);
  --fill-blocked:   rgba(255,69,58,.14);
  --fill-brand:     rgba(94,92,230,.12);
  --fill-inactive:  rgba(142,142,147,.14);

  /* —— 文字阶梯（易读基调：深）—— */
  --ink-1:#1d1d1f; --ink-2:#515156; --ink-3:#76767b; --ink-4:#b0b0b5;

  /* —— 表面（易读：实色白卡）—— */
  --surface-page:#eef1f5; --surface-card:#ffffff; --hairline:rgba(0,0,0,.08);

  /* —— 圆角 / 呼吸节奏 —— */
  --r-sm:11px; --r-md:18px; --r-lg:24px;
  --breathe-slow:4s; --breathe-mid:3.2s;
}

/* 功能元素呼吸：仅"进行中"语义使用（spec §2.2） */
@keyframes breathe-running {
  0%,100% { box-shadow: inset 0 1px 1px rgba(255,255,255,.5), 0 0 0 0 rgba(48,209,88,0); }
  50%     { box-shadow: inset 0 1px 1px rgba(255,255,255,.5), 0 0 0 3px rgba(48,209,88,.1); }
}
```

- [ ] **Step 2: 提交**

```bash
cd frontend && git add src/components/ui/tokens.css
git commit -m "feat(frontend-ui): add semantic color token CSS variables"
```

### Task 0.2：建 reset.css 并在入口引入 token

**Files:**
- Create: `frontend/src/components/ui/reset.css`
- Modify: `frontend/src/main.tsx`

- [ ] **Step 1: 读 main.tsx 现有 import 顺序**

Run: `cd frontend && sed -n '1,15p' src/main.tsx`
Expected: 看到现有 `import "./styles.css"`（或类似）的位置。

- [ ] **Step 2: 写 reset.css**

```css
/* 极小 reset —— 唯一允许裸标签全局选择器的文件之一（另一个是 tokens.css）。 */
*, *::before, *::after { box-sizing: border-box; }
body { margin: 0; -webkit-font-smoothing: antialiased; }
```

- [ ] **Step 3: 在 main.tsx 顶部、styles.css 之前引入**

在 `import "./styles.css"` 之前插入两行（变量需先于使用它的样式加载）：

```ts
import "./components/ui/tokens.css";
import "./components/ui/reset.css";
```

- [ ] **Step 4: 构建验证**

Run: `cd frontend && npm run build`
Expected: 构建成功，无 TS / CSS 错误。

- [ ] **Step 5: 提交**

```bash
cd frontend && git add src/components/ui/reset.css src/main.tsx
git commit -m "feat(frontend-ui): wire tokens.css + reset.css into entry"
```

### Task 0.3：安装 Zustand（依赖审计已过）

**Files:**
- Modify: `frontend/package.json` `frontend/package-lock.json`

- [ ] **Step 1: 安装固定版本**

Run: `cd frontend && npm install zustand@5.0.14 --save-exact`
Expected: package.json 出现 `"zustand": "5.0.14"`（精确版本，无 `^`，符合"审计过的依赖"政策）。

- [ ] **Step 2: 构建验证**

Run: `cd frontend && npm run build`
Expected: 成功。

- [ ] **Step 3: 提交**

```bash
cd frontend && git add package.json package-lock.json
git commit -m "build(frontend): add zustand 5.0.14 (audited, MIT, 0 deps)"
```

---

## 阶段 1 — 原子库 `components/ui/`

每个原子：`Xxx.tsx` + `Xxx.module.css` + `index.ts`，配一个 Vitest 测试。此阶段新组件暂未被引用，零回归风险。

### Task 1.1：StatusBadge（5 语义色 + 运行态呼吸）

**Files:**
- Create: `frontend/src/components/ui/StatusBadge/StatusBadge.tsx`
- Create: `frontend/src/components/ui/StatusBadge/StatusBadge.module.css`
- Create: `frontend/src/components/ui/StatusBadge/index.ts`
- Test: `frontend/src/components/ui/StatusBadge/StatusBadge.test.tsx`

- [ ] **Step 1: 写失败测试**

```tsx
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { StatusBadge } from "./StatusBadge";

describe("StatusBadge", () => {
  it("渲染文案与对应语义 tone 的 class", () => {
    const { container } = render(<StatusBadge tone="running">自主回复</StatusBadge>);
    expect(screen.getByText("自主回复")).toBeInTheDocument();
    // CSS Module 编译后类名带哈希，用 [class*=] 匹配语义片段
    expect(container.querySelector('[class*="running"]')).not.toBeNull();
  });

  it("held tone 不应带 running 呼吸类", () => {
    const { container } = render(<StatusBadge tone="held">暂缓</StatusBadge>);
    expect(container.querySelector('[class*="running"]')).toBeNull();
  });
});
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cd frontend && npx vitest run src/components/ui/StatusBadge -v`
Expected: FAIL —— 模块 `./StatusBadge` 不存在。

- [ ] **Step 3: 写 StatusBadge.module.css**

```css
.badge {
  display: inline-flex; align-items: center; gap: 6px;
  font-size: 11.5px; font-weight: 600; letter-spacing: -.1px;
  padding: 7px 13px; border-radius: var(--r-sm); white-space: nowrap;
  box-shadow: inset 0 1px 1px rgba(255,255,255,.5);
}
.dot { width: 6px; height: 6px; border-radius: 50%; }

.running   { color: #1a9c45; background: var(--fill-running);   border: 1px solid rgba(48,209,88,.28);  animation: breathe-running var(--breathe-mid) ease-in-out infinite; }
.running .dot   { background: var(--color-running); box-shadow: 0 0 6px 1px var(--color-running); }
.scheduled { color: var(--color-scheduled); background: var(--fill-scheduled); border: 1px solid rgba(10,132,255,.22); }
.scheduled .dot { background: var(--color-scheduled); }
.held      { color: #c47800; background: var(--fill-held);      border: 1px solid rgba(255,159,10,.30); }
.held .dot      { background: var(--color-held); }
.blocked   { color: #d6342a; background: var(--fill-blocked);   border: 1px solid rgba(255,69,58,.30); }
.blocked .dot   { background: var(--color-blocked); }
.inactive  { color: #6e6e73; background: var(--fill-inactive);  border: 1px solid rgba(142,142,147,.28); }
.inactive .dot  { background: var(--color-inactive); }
```

- [ ] **Step 4: 写 StatusBadge.tsx**

```tsx
import styles from "./StatusBadge.module.css";

export type StatusTone = "running" | "scheduled" | "held" | "blocked" | "inactive";

export function StatusBadge({ tone, children }: { tone: StatusTone; children: React.ReactNode }) {
  return (
    <span className={`${styles.badge} ${styles[tone]}`}>
      <span className={styles.dot} />
      {children}
    </span>
  );
}
```

- [ ] **Step 5: 写 index.ts**

```ts
export { StatusBadge, type StatusTone } from "./StatusBadge";
```

- [ ] **Step 6: 运行测试确认通过**

Run: `cd frontend && npx vitest run src/components/ui/StatusBadge -v`
Expected: PASS（2 个用例）。

- [ ] **Step 7: 提交**

```bash
cd frontend && git add src/components/ui/StatusBadge
git commit -m "feat(frontend-ui): add StatusBadge atom (5 semantic tones)"
```

### Task 1.2：MetricCard（从 App.tsx 复刻签名）

**Files:**
- Create: `frontend/src/components/ui/MetricCard/MetricCard.tsx`
- Create: `frontend/src/components/ui/MetricCard/MetricCard.module.css`
- Create: `frontend/src/components/ui/MetricCard/index.ts`
- Test: `frontend/src/components/ui/MetricCard/MetricCard.test.tsx`

> 现有签名（App.tsx:4626）：`{ detail, label, onClick, value }`，渲染 `<button class="metricCard">`。保持 props 形状不变，只把样式搬进 module.css，便于后续替换调用点。

- [ ] **Step 1: 写失败测试**

```tsx
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { MetricCard } from "./MetricCard";

describe("MetricCard", () => {
  it("渲染 label/value/detail 并响应点击", () => {
    const onClick = vi.fn();
    render(<MetricCard label="Managed Users" value={128} detail="Agent 运营好友" onClick={onClick} />);
    expect(screen.getByText("Managed Users")).toBeInTheDocument();
    expect(screen.getByText("128")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button"));
    expect(onClick).toHaveBeenCalledOnce();
  });
});
```

- [ ] **Step 2: 运行确认失败**

Run: `cd frontend && npx vitest run src/components/ui/MetricCard -v`
Expected: FAIL —— 模块不存在。

- [ ] **Step 3: 写 MetricCard.module.css**

```css
.card {
  display: flex; flex-direction: column; gap: 4px;
  background: var(--surface-card); border: 1px solid var(--hairline);
  border-radius: var(--r-md); padding: 20px 22px; cursor: pointer; text-align: left;
  box-shadow: 0 14px 34px -24px rgba(20,30,60,.3), inset 0 1px 1px rgba(255,255,255,.9);
  transition: transform .18s ease;
}
.card:hover { transform: translateY(-3px); }
.label { font-size: 12.5px; color: var(--ink-2); font-weight: 500; }
.value { font-size: 40px; color: var(--ink-1); font-weight: 680; letter-spacing: -.8px; line-height: 1; }
.detail { font-size: 11.5px; color: var(--ink-3); }
```

- [ ] **Step 4: 写 MetricCard.tsx**

```tsx
import styles from "./MetricCard.module.css";

export function MetricCard({ detail, label, onClick, value }: {
  detail: string; label: string; onClick: () => void; value: number;
}) {
  return (
    <button className={styles.card} onClick={onClick}>
      <span className={styles.label}>{label}</span>
      <strong className={styles.value}>{value}</strong>
      <small className={styles.detail}>{detail}</small>
    </button>
  );
}
```

- [ ] **Step 5: 写 index.ts**

```ts
export { MetricCard } from "./MetricCard";
```

- [ ] **Step 6: 运行确认通过**

Run: `cd frontend && npx vitest run src/components/ui/MetricCard -v`
Expected: PASS。

- [ ] **Step 7: 提交**

```bash
cd frontend && git add src/components/ui/MetricCard
git commit -m "feat(frontend-ui): add MetricCard atom"
```

### Task 1.3：其余原子（StatusLine / PlanStep / EmptyState / Avatar）

> 同 1.1/1.2 的模式：复刻 App.tsx 现有签名（StatusLine 4646 `{label,tone,value}` tone=`"ai"|"good"|"neutral"|"warn"`；PlanStep 4655 `{detail,status,title}` status=`"ready"|"pending"`；EmptyState 4676 `{icon,title,hint,action}`），样式搬进各自 module.css，色值改用 token 变量。Avatar 为新原子：`{ name, tone }`，tone 复用 `StatusTone`，圆角玻璃头像 + `.live` 呼吸环。

**Files（每个组件一套）:**
- Create: `frontend/src/components/ui/StatusLine/{StatusLine.tsx,StatusLine.module.css,index.ts}`
- Create: `frontend/src/components/ui/PlanStep/{PlanStep.tsx,PlanStep.module.css,index.ts}`
- Create: `frontend/src/components/ui/EmptyState/{EmptyState.tsx,EmptyState.module.css,index.ts}`
- Create: `frontend/src/components/ui/Avatar/{Avatar.tsx,Avatar.module.css,index.ts}`
- Test: 各 `*.test.tsx`

- [ ] **Step 1: 写 Avatar 失败测试（其余组件各写一个"渲染+class"冒烟测试，与 1.1 同构）**

```tsx
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Avatar } from "./Avatar";

describe("Avatar", () => {
  it("渲染姓名首字与 tone class", () => {
    const { container } = render(<Avatar name="陈先生" tone="running" />);
    expect(screen.getByText("陈")).toBeInTheDocument();
    expect(container.querySelector('[class*="running"]')).not.toBeNull();
  });
});
```

- [ ] **Step 2: 运行确认失败**

Run: `cd frontend && npx vitest run src/components/ui/Avatar -v`
Expected: FAIL。

- [ ] **Step 3: 写 Avatar.module.css**

```css
.avatar {
  width: 40px; height: 40px; border-radius: 13px; flex-shrink: 0;
  display: flex; align-items: center; justify-content: center; position: relative;
  font-size: 13.5px; font-weight: 600; color: #fff; border: 1px solid rgba(255,255,255,.5);
  box-shadow: inset 0 1.5px 1px rgba(255,255,255,.6), 0 6px 14px -6px rgba(20,30,60,.3);
}
.running   { background: linear-gradient(135deg,#34d35e,#1fa84a); }
.scheduled { background: linear-gradient(135deg,#3a9bff,#0a6ff0); }
.held      { background: linear-gradient(135deg,#ffb43a,#f5920a); }
.blocked   { background: linear-gradient(135deg,#ff6961,#e5392e); }
.inactive  { background: linear-gradient(135deg,#aeaeb2,#7c7c80); }
```

- [ ] **Step 4: 写 Avatar.tsx**

```tsx
import styles from "./Avatar.module.css";
import type { StatusTone } from "../StatusBadge";

export function Avatar({ name, tone = "inactive" }: { name: string; tone?: StatusTone }) {
  return <div className={`${styles.avatar} ${styles[tone]}`}>{name.slice(0, 1)}</div>;
}
```

- [ ] **Step 5: 写 index.ts（Avatar）**

```ts
export { Avatar } from "./Avatar";
```

- [ ] **Step 6: 复刻 StatusLine / PlanStep / EmptyState**

对这三个组件，分别从 App.tsx 复制 JSX 结构（StatusLine@4646、PlanStep@4655、EmptyState@4676），把 `className="statusLine ${tone}"` 等改为 `className={\`${styles.line} ${styles[tone]}\`}`，并新建对应 module.css（从 styles.css 摘出 `.statusLine`/`.planStep`/`.emptyState` 规则、色值改 token 变量）。每个配一个与 Step 1 同构的冒烟测试。

- [ ] **Step 7: 运行全部 ui 测试确认通过**

Run: `cd frontend && npx vitest run src/components/ui -v`
Expected: 全部 PASS。

- [ ] **Step 8: 构建验证**

Run: `cd frontend && npm run build`
Expected: 成功。

- [ ] **Step 9: 提交**

```bash
cd frontend && git add src/components/ui
git commit -m "feat(frontend-ui): add StatusLine/PlanStep/EmptyState/Avatar atoms"
```

---

## 阶段 2 — store + Shell 外壳

### Task 2.1：抽共享类型到 `types/`

**Files:**
- Create: `frontend/src/types/index.ts`
- Modify: `frontend/src/App.tsx`（删除被移走的类型定义，改为 import）

- [ ] **Step 1: 定位类型定义**

Run: `cd frontend && grep -nE "^type (Channel|AgentStatus|ContactTab)|^interface (Account|Contact|Message|EventItem|TaskItem)" src/App.tsx`
Expected: 列出 `Channel`(59)、`AgentStatus`(58)、`ContactTab`(60) 等行号。

- [ ] **Step 2: 建 types/index.ts，迁移 Channel 与核心实体类型**

把 `Channel`、`AgentStatus`、`ContactTab`、`Account`、`Contact`、`Message`、`EventItem`、`TaskItem` 的定义从 App.tsx 剪切到此文件并 `export`。示例（以实际定义为准，逐字搬运）：

```ts
export type Channel =
  | "command" | "overview" | "userOps" | "groupOps" | "momentOps"
  | "content" | "systemStrategy" | "operations" | "autonomy"
  | "evolution" | "quality" | "llmProviders" | "knowledgeWiki";
export type AgentStatus = "normal" | "managed";
export type ContactTab = "all" | "managed" | "normal";
// Account / Contact / Message / EventItem / TaskItem：逐字从 App.tsx 搬运
```

- [ ] **Step 3: App.tsx 改为 import**

在 App.tsx 顶部加 `import type { Channel, AgentStatus, ContactTab, Account, Contact, Message, EventItem, TaskItem } from "./types";`，删掉原定义。

- [ ] **Step 4: 构建验证**

Run: `cd frontend && npm run build`
Expected: 成功（类型搬运无遗漏则零错误）。

- [ ] **Step 5: 提交**

```bash
cd frontend && git add src/types/index.ts src/App.tsx
git commit -m "refactor(frontend): extract shared types to types/index.ts"
```

### Task 2.2：navigationStore

**Files:**
- Create: `frontend/src/stores/navigationStore.ts`
- Test: `frontend/src/stores/navigationStore.test.ts`

- [ ] **Step 1: 写失败测试**

```ts
import { describe, expect, it, beforeEach } from "vitest";
import { useNavigationStore } from "./navigationStore";

describe("navigationStore", () => {
  beforeEach(() => useNavigationStore.setState({ activeChannel: "command" }));
  it("默认 channel 为 command", () => {
    expect(useNavigationStore.getState().activeChannel).toBe("command");
  });
  it("setChannel 切换 activeChannel", () => {
    useNavigationStore.getState().setChannel("userOps");
    expect(useNavigationStore.getState().activeChannel).toBe("userOps");
  });
});
```

- [ ] **Step 2: 运行确认失败**

Run: `cd frontend && npx vitest run src/stores/navigationStore -v`
Expected: FAIL —— 模块不存在。

- [ ] **Step 3: 写 navigationStore.ts**

```ts
import { create } from "zustand";
import type { Channel } from "../types";

interface NavigationState {
  activeChannel: Channel;
  setChannel: (channel: Channel) => void;
}

export const useNavigationStore = create<NavigationState>((set) => ({
  activeChannel: "command",
  setChannel: (channel) => set({ activeChannel: channel }),
}));
```

- [ ] **Step 4: 运行确认通过**

Run: `cd frontend && npx vitest run src/stores/navigationStore -v`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
cd frontend && git add src/stores/navigationStore.ts src/stores/navigationStore.test.ts
git commit -m "feat(frontend): add navigationStore (zustand)"
```

### Task 2.3：accountStore（含 localStorage 持久化 + 派生）

**Files:**
- Create: `frontend/src/stores/accountStore.ts`
- Test: `frontend/src/stores/accountStore.test.ts`

> 复刻 App.tsx 现有逻辑：`selectedAccountId` 初值 `localStorage.getItem("wechatagent.accountId") || ""`；`currentAccountId` = 若 selected 在 accounts 中则用之，否则回退首个；`onlineCount` = online 账号数。

- [ ] **Step 1: 写失败测试**

```ts
import { describe, expect, it, beforeEach } from "vitest";
import { useAccountStore } from "./accountStore";
import type { Account } from "../types";

const acc = (id: string, online = true): Account =>
  ({ id, accountId: id, online } as Account);

describe("accountStore", () => {
  beforeEach(() => useAccountStore.setState({ accounts: [], selectedAccountId: "" }));

  it("currentAccountId 在 selected 无效时回退首个账号", () => {
    useAccountStore.getState().setAccounts([acc("a"), acc("b")]);
    expect(useAccountStore.getState().currentAccountId()).toBe("a");
  });

  it("onlineCount 统计在线账号数", () => {
    useAccountStore.getState().setAccounts([acc("a", true), acc("b", false)]);
    expect(useAccountStore.getState().onlineCount()).toBe(1);
  });

  it("selectAccount 写入并持久化", () => {
    useAccountStore.getState().setAccounts([acc("a"), acc("b")]);
    useAccountStore.getState().selectAccount("b");
    expect(useAccountStore.getState().currentAccountId()).toBe("b");
    expect(localStorage.getItem("wechatagent.accountId")).toBe("b");
  });
});
```

- [ ] **Step 2: 运行确认失败**

Run: `cd frontend && npx vitest run src/stores/accountStore -v`
Expected: FAIL。

- [ ] **Step 3: 写 accountStore.ts**

```ts
import { create } from "zustand";
import type { Account } from "../types";

const STORAGE_KEY = "wechatagent.accountId";

interface AccountState {
  accounts: Account[];
  selectedAccountId: string;
  setAccounts: (accounts: Account[]) => void;
  selectAccount: (accountId: string) => void;
  currentAccountId: () => string;
  currentAccount: () => Account | undefined;
  onlineCount: () => number;
}

export const useAccountStore = create<AccountState>((set, get) => ({
  accounts: [],
  selectedAccountId: localStorage.getItem(STORAGE_KEY) || "",
  setAccounts: (accounts) => set({ accounts }),
  selectAccount: (accountId) => {
    localStorage.setItem(STORAGE_KEY, accountId);
    set({ selectedAccountId: accountId });
  },
  currentAccountId: () => {
    const { accounts, selectedAccountId } = get();
    return accounts.some((a) => a.accountId === selectedAccountId)
      ? selectedAccountId
      : accounts[0]?.accountId ?? "";
  },
  currentAccount: () => {
    const id = get().currentAccountId();
    return get().accounts.find((a) => a.accountId === id);
  },
  onlineCount: () => get().accounts.filter((a) => a.online).length,
}));
```

- [ ] **Step 4: 运行确认通过**

Run: `cd frontend && npx vitest run src/stores/accountStore -v`
Expected: PASS（3 用例）。

- [ ] **Step 5: 提交**

```bash
cd frontend && git add src/stores/accountStore.ts src/stores/accountStore.test.ts
git commit -m "feat(frontend): add accountStore with persistence + derived selectors"
```

### Task 2.4：contactStore + uiStore

**Files:**
- Create: `frontend/src/stores/contactStore.ts`
- Create: `frontend/src/stores/uiStore.ts`
- Test: `frontend/src/stores/contactStore.test.ts`

> contactStore 复刻 App.tsx：`managedCount` = agentStatus==="managed" 数；`normalCount` = contacts.length - managedCount。

- [ ] **Step 1: 写 contactStore 失败测试**

```ts
import { describe, expect, it, beforeEach } from "vitest";
import { useContactStore } from "./contactStore";
import type { Contact } from "../types";

const c = (id: string, managed: boolean): Contact =>
  ({ id, agentStatus: managed ? "managed" : "normal" } as Contact);

describe("contactStore", () => {
  beforeEach(() => useContactStore.setState({ contacts: [], selected: null, contactTab: "all" }));
  it("managedCount / normalCount 派生正确", () => {
    useContactStore.getState().setContacts([c("a", true), c("b", false), c("d", true)]);
    expect(useContactStore.getState().managedCount()).toBe(2);
    expect(useContactStore.getState().normalCount()).toBe(1);
  });
});
```

- [ ] **Step 2: 运行确认失败**

Run: `cd frontend && npx vitest run src/stores/contactStore -v`
Expected: FAIL。

- [ ] **Step 3: 写 contactStore.ts**

```ts
import { create } from "zustand";
import type { Contact, ContactTab } from "../types";

interface ContactState {
  contacts: Contact[];
  selected: Contact | null;
  contactTab: ContactTab;
  setContacts: (contacts: Contact[]) => void;
  setSelected: (c: Contact | null) => void;
  setContactTab: (t: ContactTab) => void;
  managedCount: () => number;
  normalCount: () => number;
}

export const useContactStore = create<ContactState>((set, get) => ({
  contacts: [],
  selected: null,
  contactTab: "all",
  setContacts: (contacts) => set({ contacts }),
  setSelected: (selected) => set({ selected }),
  setContactTab: (contactTab) => set({ contactTab }),
  managedCount: () => get().contacts.filter((c) => c.agentStatus === "managed").length,
  normalCount: () => get().contacts.length - get().managedCount(),
}));
```

- [ ] **Step 4: 写 uiStore.ts**

```ts
import { create } from "zustand";

interface UiState {
  busy: boolean;
  error: string;
  setBusy: (busy: boolean) => void;
  setError: (error: string) => void;
}

export const useUiStore = create<UiState>((set) => ({
  busy: false,
  error: "",
  setBusy: (busy) => set({ busy }),
  setError: (error) => set({ error }),
}));
```

- [ ] **Step 5: 运行确认通过 + 构建**

Run: `cd frontend && npx vitest run src/stores -v && npm run build`
Expected: 全 PASS + 构建成功。

- [ ] **Step 6: 提交**

```bash
cd frontend && git add src/stores/contactStore.ts src/stores/uiStore.ts src/stores/contactStore.test.ts
git commit -m "feat(frontend): add contactStore + uiStore"
```

### Task 2.5：channels.ts 注册表（先含 overview 一个真实 Component）

**Files:**
- Create: `frontend/src/app/channels.ts`
- Create: `frontend/src/features/overview/index.tsx`（占位实现，下个任务填真实内容）

> 现有元数据来源：`channels` 数组（App.tsx:670，含 id/label/caption/icon）、`channelEyebrow`(5035)/`channelTitle`(5006)/`channelSubtitle`(5064)。把它们合并进单一注册表。

- [ ] **Step 1: 建 overview feature 占位入口**

`frontend/src/features/overview/index.tsx`：

```tsx
export default function OverviewFeature() {
  return <section>overview placeholder</section>;
}
```

- [ ] **Step 2: 写 channels.ts**

把 App.tsx 的 `channels` 数组 + 三个文案函数合并。先只给 `overview` 接真实 lazy Component，其余 12 个 channel 暂时也指向 overview 占位（下一阶段逐个替换）。逐字搬运 label/caption/icon 与 eyebrow/title/subtitle 文案：

```ts
import { lazy } from "react";
import {
  BrainCircuit, LayoutDashboard, UserRoundCheck, UsersRound, Sparkles,
  FileText, Settings2, Bot, Activity, ShieldCheck, Workflow, FileBox,
  type LucideIcon,
} from "lucide-react";
import type { Channel } from "../types";

const OverviewFeature = lazy(() => import("../features/overview"));

export interface ChannelDef {
  id: Channel;
  group: "运营" | "知识" | "系统";
  label: string;
  caption: string;
  icon: LucideIcon;
  eyebrow: string;
  title: string;
  subtitle: string;
  Component: React.LazyExoticComponent<React.ComponentType>;
}

export const CHANNELS: ChannelDef[] = [
  { id: "overview", group: "运营", label: "工作台", caption: "运行态势", icon: LayoutDashboard,
    eyebrow: /* 逐字搬 channelEyebrow("overview") */ "RUNTIME",
    title: /* 逐字搬 channelTitle("overview") */ "运行态势",
    subtitle: /* 逐字搬 channelSubtitle("overview") */ "",
    Component: OverviewFeature },
  // 其余 12 个 channel：先用 OverviewFeature 占位，label/caption/icon/文案逐字搬运
];
```

- [ ] **Step 3: 构建验证**

Run: `cd frontend && npm run build`
Expected: 成功。

- [ ] **Step 4: 提交**

```bash
cd frontend && git add src/app/channels.ts src/features/overview/index.tsx
git commit -m "feat(frontend): add channels registry (overview wired, rest placeholder)"
```

### Task 2.6：Shell 外壳（跑通 overview，与旧 App 并存）

**Files:**
- Create: `frontend/src/app/Shell.tsx`
- Create: `frontend/src/app/Shell.module.css`
- Test: `frontend/src/app/Shell.test.tsx`

> 此任务 Shell 暂不接管 main.tsx —— 先独立可测，验证骨架，下个阶段搬完 feature 再切换入口。

- [ ] **Step 1: 写失败测试**

```tsx
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Shell } from "./Shell";
import { useNavigationStore } from "../stores/navigationStore";

describe("Shell", () => {
  it("默认渲染侧栏所有 channel 标签", async () => {
    useNavigationStore.setState({ activeChannel: "overview" });
    render(<Shell />);
    expect(await screen.findByText("工作台")).toBeInTheDocument();
    expect(screen.getByText("用户运营")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 运行确认失败**

Run: `cd frontend && npx vitest run src/app/Shell -v`
Expected: FAIL。

- [ ] **Step 3: 写 Shell.module.css**

```css
.shell { display: flex; min-height: 100vh; background: var(--surface-page); }
.side { width: 250px; flex-shrink: 0; padding: 28px 16px; border-right: 1px solid var(--hairline); background: var(--surface-card); }
.nav { display: flex; flex-direction: column; gap: 2px; }
.channel { display: flex; align-items: center; gap: 11px; padding: 9px 12px; border-radius: var(--r-sm);
  font-size: 13.5px; color: var(--ink-2); font-weight: 500; cursor: pointer; background: none; border: none; text-align: left; }
.channel:hover { background: rgba(0,0,0,.03); color: var(--ink-1); }
.active { color: var(--ink-1); font-weight: 600; background: rgba(10,132,255,.08); }
.main { flex: 1; padding: 32px 44px; overflow: hidden; }
.header { margin-bottom: 24px; }
.eyebrow { font-size: 11px; color: var(--color-brand); font-weight: 600; letter-spacing: .9px; text-transform: uppercase; }
.title { font-size: 32px; color: var(--ink-1); font-weight: 700; letter-spacing: -1.2px; margin: 8px 0 4px; }
.subtitle { font-size: 14px; color: var(--ink-2); }
.skeleton { padding: 40px; color: var(--ink-3); }
```

- [ ] **Step 4: 写 Shell.tsx**

```tsx
import { Suspense } from "react";
import { CHANNELS } from "./channels";
import { useNavigationStore } from "../stores/navigationStore";
import styles from "./Shell.module.css";

export function Shell() {
  const activeChannel = useNavigationStore((s) => s.activeChannel);
  const setChannel = useNavigationStore((s) => s.setChannel);
  const def = CHANNELS.find((c) => c.id === activeChannel) ?? CHANNELS[0];
  const { Component } = def;

  return (
    <div className={styles.shell}>
      <aside className={styles.side}>
        <nav className={styles.nav} aria-label="Product channels">
          {CHANNELS.map((c) => {
            const Icon = c.icon;
            return (
              <button
                key={c.id}
                className={`${styles.channel} ${c.id === activeChannel ? styles.active : ""}`}
                onClick={() => setChannel(c.id)}
              >
                <Icon size={17} />
                <span>{c.label}</span>
              </button>
            );
          })}
        </nav>
      </aside>
      <main className={styles.main}>
        <header className={styles.header}>
          <p className={styles.eyebrow}>{def.eyebrow}</p>
          <h1 className={styles.title}>{def.title}</h1>
          <span className={styles.subtitle}>{def.subtitle}</span>
        </header>
        <Suspense fallback={<div className={styles.skeleton}>加载中…</div>}>
          <Component />
        </Suspense>
      </main>
    </div>
  );
}
```

- [ ] **Step 5: 运行确认通过 + 构建**

Run: `cd frontend && npx vitest run src/app/Shell -v && npm run build`
Expected: PASS + 构建成功。

- [ ] **Step 6: 提交**

```bash
cd frontend && git add src/app/Shell.tsx src/app/Shell.module.css src/app/Shell.test.tsx
git commit -m "feat(frontend): add Shell scaffold (sidebar + header + lazy slot)"
```

---

## 阶段 3 — 按 feature 逐个搬迁

**每个 feature 的通用流程（对每个 channel 重复）：**
1. 建 `features/<domain>/index.tsx` + 子组件 + `*.module.css`。
2. 从 App.tsx 复制对应 View 函数体；props 改为从 store 选择器读取（不再透传）。
3. 从 styles.css 摘出该 feature 独有的样式类到 module.css，色值改 token 变量。
4. 在 channels.ts 把该 channel 的 `Component` 指向真实 feature。
5. `npm run build` + `npm test` + 浏览器点开该 channel 走主路径。
6. 提交（一个 feature 一个提交）。

下面给出**第一个完整样板（overview）**，其余 feature 按同一模式执行。

### Task 3.1：overview feature（完整样板）

**Files:**
- Modify: `frontend/src/features/overview/index.tsx`（替换占位为真实实现）
- Create: `frontend/src/features/overview/Overview.module.css`
- Test: `frontend/src/features/overview/overview.test.tsx`

> 来源：App.tsx `OverviewView`(1790)。它现在收 `contacts/managedCount/normalCount/onlineCount/pendingTasks/latestEvent/onOpenChannel` 7 个 props。重构后：前四个从 store 读，`onOpenChannel` 用 navigationStore.setChannel，`pendingTasks`/`latestEvent` 暂作为 props 由 user-ops/operations 迁移后接入（此阶段先用 store 能提供的，pendingTasks/latestEvent 暂传 0/undefined 占位并标注 TODO 关联任务）。

- [ ] **Step 1: 写失败测试**

```tsx
import { render, screen } from "@testing-library/react";
import { describe, expect, it, beforeEach } from "vitest";
import OverviewFeature from "./index";
import { useContactStore } from "../../stores/contactStore";
import { useAccountStore } from "../../stores/accountStore";
import type { Contact } from "../../types";

describe("OverviewFeature", () => {
  beforeEach(() => {
    useContactStore.setState({ contacts: [
      { id: "a", agentStatus: "managed" } as Contact,
      { id: "b", agentStatus: "normal" } as Contact,
    ], selected: null, contactTab: "all" });
    useAccountStore.setState({ accounts: [], selectedAccountId: "" });
  });
  it("显示 managedCount 指标卡数值", () => {
    render(<OverviewFeature />);
    expect(screen.getByText("Managed Users")).toBeInTheDocument();
    expect(screen.getByText("1")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 运行确认失败**

Run: `cd frontend && npx vitest run src/features/overview -v`
Expected: FAIL（占位实现没有 Managed Users 文案）。

- [ ] **Step 3: 写 Overview.module.css**

从 styles.css 摘 `.overviewGrid`/`.widePanel`/`.sidePanel`/`.panelHead`/`.principleGrid`/`.eventPreview` 规则，色值改 token 变量。（逐字搬运现有规则，把 `#xxx` 换 `var(--ink-*)`/`var(--surface-*)`。）

- [ ] **Step 4: 写真实 index.tsx**

```tsx
import { Activity, Sparkles } from "lucide-react";
import { MetricCard } from "../../components/ui/MetricCard";
import { EmptyState } from "../../components/ui/EmptyState";
import { useContactStore } from "../../stores/contactStore";
import { useAccountStore } from "../../stores/accountStore";
import { useNavigationStore } from "../../stores/navigationStore";
import styles from "./Overview.module.css";

export default function OverviewFeature() {
  const contacts = useContactStore((s) => s.contacts);
  const managedCount = useContactStore((s) => s.managedCount());
  const normalCount = useContactStore((s) => s.normalCount());
  const onlineCount = useAccountStore((s) => s.onlineCount());
  const setChannel = useNavigationStore((s) => s.setChannel);

  // TODO(user-ops/operations 迁移后接入): pendingTasks / latestEvent
  const pendingTasks = 0;

  return (
    <section className={styles.overviewGrid}>
      <MetricCard label="Managed Users" value={managedCount} detail="Agent 运营好友" onClick={() => setChannel("userOps")} />
      <MetricCard label="Contact Base" value={contacts.length} detail={`${normalCount} 普通好友`} onClick={() => setChannel("userOps")} />
      <MetricCard label="Account Online" value={onlineCount} detail="可用微信账号" onClick={() => setChannel("overview")} />
      <MetricCard label="Pending Tasks" value={pendingTasks} detail="待执行任务" onClick={() => setChannel("operations")} />
      {/* widePanel / sidePanel：逐字搬 OverviewView 的 JSX，className 改 styles.* */}
    </section>
  );
}
```

- [ ] **Step 5: 运行确认通过**

Run: `cd frontend && npx vitest run src/features/overview -v`
Expected: PASS。

- [ ] **Step 6: 从 App.tsx 删除 OverviewView 并清理调用点**

删 App.tsx 的 `OverviewView` 函数定义；删 `{activeChannel === "overview" && <OverviewView .../>}` 块（旧 App 仍在跑其他 channel，此 channel 已由新结构接管，但旧 App 尚未切换为 Shell —— 见 Task 4.1 统一切换；在那之前 overview 在旧 App 里暂时空白，可接受，因为下一阶段会整体切到 Shell）。

> 说明：为避免"删一半两边都坏"，本阶段每个 feature 的 App.tsx 删除只删 View 定义，调用点的整体替换集中在 Task 4.1 切换入口时一次性完成。若希望每步都能在旧 App 看到效果，可选择在 Task 4.1 先行：先切 Shell 入口，再逐个 feature 搬。两种顺序都可，推荐先搬 feature 再切入口（当前顺序）。

- [ ] **Step 7: 构建 + 提交**

```bash
cd frontend && npm run build && git add src/features/overview src/app/channels.ts src/App.tsx
git commit -m "refactor(frontend): migrate overview to feature module + store"
```

### Task 3.2 ~ 3.13：其余 feature（同 3.1 模式）

按推荐顺序逐个迁移，每个一个独立提交。每个任务的"来源 View / 现有 props / 目标 store"如下表，执行时套用 3.1 的 6 步流程：

| # | feature 目录 | channel id | 来源 View(行号) | 现有 props → 重构后来源 |
| --- | --- | --- | --- | --- |
| 3.2 | `command-center` | command | `CommandCenterView`(1650) | accounts/souls/currentAccount→accountStore；managedCount/onlineCount→contact/accountStore；command* 留 feature 内部 useState |
| 3.3 | `content-assets` | content | `ContentAssetsView`(2776) | assets/assetDraft 留 feature 内部 |
| 3.4 | `operations` | operations | `OperationsView`(2609) | tasks → feature hook；派生 pendingTasks 在此 feature 计算并写回（供 overview 用，见下注） |
| 3.5 | `user-ops` | userOps | `UserOperationCockpit`(1876)+`ContactsView`(2510)+`MemoryDrawer`(12167)+`SimulationResult`(2399) 等 | contacts/selected/contactTab→contactStore；messages/events/memory*/simulation*/guide* 留 feature hook |
| 3.6 | `system-strategy` | systemStrategy | `SystemStrategyView`(3563)+`StatePolicyAdmin`(3617)+`TaxonomiesAdmin`(3706)+`DomainConfigEditor`(2973)+`UserPlaybookPanel`(3171)+`DomainPromptPanel`(3353) | souls/promptTemplates/operationDomains/domainDrafts/playbooks/*Draft/editing*Id 留 feature |
| 3.7 | `llm-providers` | llmProviders | `LlmProvidersView`(4112) | feature 内部状态 |
| 3.8 | `autonomy` | autonomy | `AutonomyLoopView`(5669) | accountId←accountStore.currentAccountId |
| 3.9 | `evolution` | evolution | `EvolutionCenterView`(5687)（复用现有 `EvolutionCenterTab.tsx`） | 已独立，包装为 feature 入口即可 |
| 3.10 | `quality` | quality | `QualityCenterView`(12246)+`OutcomeMetricsTab`(12279) | accountId←accountStore |
| 3.11 | `knowledge` | knowledgeWiki | `KnowledgeWikiView`(5750) 及其大量子组件（`ChunkGraphView`6046/`DocumentsView`6499/`ImportWizard`6733/`ReviewView`8111/`LintView`7863/`AskView`7462 等） | 最大块，**子组件再拆 `features/knowledge/components/`**，feature 内部状态 + hooks |
| 3.12 | `group-ops` | groupOps | （占位页，Phase 1 不实现）保留空 feature + "规划中"提示 | 无 |
| 3.13 | `moment-ops` | momentOps | （占位页，Phase 1 不实现）保留空 feature + "规划中"提示 | 无 |

> **pendingTasks / latestEvent 跨 feature 派生**：operations 迁移（3.4）时把 `pendingTasks` 派生暴露（可加到 uiStore 或新建 `operationsStore`），user-ops 迁移（3.5）时把 `latestEvent` 暴露。完成后回到 `features/overview/index.tsx` 删掉 Step 4 的 TODO 占位、接真实值，并补一次提交。

- [ ] **Step（每个 feature）**: 套用 Task 3.1 的 6 步（建模块 → 测试 → 摘样式 → 接 store → channels 接线 → 删 App.tsx 对应 View → build/test/浏览器验证 → 提交）。
- [ ] **Step（knowledge 特殊）**: 3.11 因体量大，**先把子组件分文件搬到 `features/knowledge/components/`**（每个子组件一个提交），最后再组装 `index.tsx`。
- [ ] **Step（overview 回填）**: 3.4 与 3.5 完成后，回填 overview 的 pendingTasks/latestEvent，提交 `fix(frontend): wire overview pendingTasks/latestEvent from stores`。

---

## 阶段 4 — 切换入口 + 收尾

### Task 4.1：main.tsx / App.tsx 切换到 Shell

**Files:**
- Modify: `frontend/src/App.tsx`
- Create: `frontend/src/app/GlobalErrorBanner.tsx`

> 前置：阶段 3 所有 feature 已搬完且 channels.ts 全部指向真实 Component。

- [ ] **Step 1: 建 GlobalErrorBanner**

```tsx
import { useUiStore } from "../stores/uiStore";

export function GlobalErrorBanner() {
  const error = useUiStore((s) => s.error);
  if (!error) return null;
  return <div role="alert" style={{ /* 简单错误条，样式可后续移 module.css */ }}>{error}</div>;
}
```

- [ ] **Step 2: App.tsx 瘦身**

把 App.tsx 整个函数体替换为：

```tsx
import { Shell } from "./app/Shell";
import { GlobalErrorBanner } from "./app/GlobalErrorBanner";
// 启动数据预加载移到 Shell 的 useEffect 或各 store 初始化

export function App() {
  return (
    <>
      <GlobalErrorBanner />
      <Shell />
    </>
  );
}
```

把原 App() 里的启动数据加载（`loadAll`/`syncAccounts`/`accounts` 拉取）迁移到 Shell 的 `useEffect` 中，调用 store 的 setter（`useAccountStore.getState().setAccounts(...)` 等）。

- [ ] **Step 3: 构建 + 全量测试**

Run: `cd frontend && npm run build && npm test`
Expected: 成功；现有测试（EvolutionCenterTab/AutonomyOutcomes）不回归。

- [ ] **Step 4: 浏览器全 channel 回归**

Run: `cd frontend && npm run dev`（另需后端 `cargo run` 提供 /api）
逐个点击 13 个 channel，确认渲染、数据加载、切换正常。

- [ ] **Step 5: 提交**

```bash
cd frontend && git add src/App.tsx src/app/GlobalErrorBanner.tsx
git commit -m "refactor(frontend): switch entry to Shell, slim App.tsx to ~80 lines"
```

### Task 4.2：清理旧 styles.css 残留

**Files:**
- Modify: `frontend/src/styles.css`

- [ ] **Step 1: 找出仍被引用的全局类**

Run: `cd frontend && grep -rEo 'className="[a-zA-Z][a-zA-Z0-9 ]*"' src/App.tsx src/app | head -40`
Expected: 理想情况几乎为空（都已 module 化）。列出仍存在的全局类名。

- [ ] **Step 2: 删除已无引用的规则**

对 styles.css 里不再被任何文件引用的类（用 grep 确认每个类名在 src 下无 className 引用），逐段删除。保留仍被引用的（如尚未拆的全局工具类）。

- [ ] **Step 3: 构建 + 浏览器抽查**

Run: `cd frontend && npm run build`
Expected: 成功；浏览器抽查 3~4 个 channel 无样式丢失。

- [ ] **Step 4: 提交**

```bash
cd frontend && git add src/styles.css
git commit -m "chore(frontend): drop dead global styles after module migration"
```

### Task 4.3：no-human-takeover 红线自检 + 最终基线

**Files:** 无（校验）

- [ ] **Step 1: 红线 lint**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent" && bash scripts/check-no-human-takeover.sh`
Expected: 通过（前端新增行无禁用词）。

- [ ] **Step 2: 前端全量构建 + 测试**

Run: `cd frontend && npm run build && npm test`
Expected: 全绿。

- [ ] **Step 3: 行数验收**

Run: `cd frontend && wc -l src/App.tsx src/styles.css`
Expected: `App.tsx` ≤ 150 行（目标 ~80）；`styles.css` 仅剩 tokens/reset 等价残留。

- [ ] **Step 4: 提交（若有清理）**

```bash
cd frontend && git add -A && git commit -m "chore(frontend): final baseline check for UI refactor"
```

---

## Self-Review 检查结果

- **Spec 覆盖**：§2.1 易读基调→tokens.css 实色表面 + ink 阶梯（0.1）；§2.2 呼吸→breathe-running keyframe + StatusBadge.running（0.1/1.1）；§2.3 6 色 token→0.1；§3.1 目录→文件结构表 + 各阶段；§3.2 store→2.2~2.4；§3.3 CSS Modules→全程 module.css + 0.1 变量；§3.4 channels/Shell→2.5/2.6；§4 迁移路径→阶段 0~4 顺序与验证门；§6 Zustand→0.3；§7 验收→4.3。无遗漏。
- **占位符扫描**：feature 表（3.2~3.13）给出行号与 store 映射、套用 3.1 完整样板，非空泛 TODO；overview 的 pendingTasks/latestEvent 占位有明确回填任务（阶段 3 末尾 Step）。
- **类型一致性**：`StatusTone` 在 StatusBadge 定义、Avatar 复用；store 方法名（`setChannel`/`setAccounts`/`selectAccount`/`currentAccountId`/`managedCount`）在测试与实现、Shell/feature 调用处一致；`Channel` 来自 `types/index.ts` 单一来源。

---

**Plan complete and saved to `docs/superpowers/plans/2026-06-04-frontend-ui-refactor.md`. Two execution options:**

**1. Subagent-Driven (recommended)** — 每个任务派一个全新 subagent 执行，任务之间我来 review，迭代快、上下文干净。

**2. Inline Execution** — 在当前会话里按 executing-plans 批量执行，带检查点暂停给你 review。

**选哪种？**

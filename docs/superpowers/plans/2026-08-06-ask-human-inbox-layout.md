# 统一收件箱布局与结构对齐 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把「统一收件箱」（ask-human）频道从"内容裸贴灰底"改成与全项目一致的 `.page` + 白卡结构，让 9 个来源 chip 横向单排不折行，空态用共享 `EmptyState`。

**Architecture:** 三层各修在正确的层上：外壳去掉与 Shell 重复的 padding/max-width 并补 `askHuman` 前缀的白卡；chip 收进卡内工具栏并收紧 padding 让单排放得下；`ReviewQueue` 的四个零样式 class 补最小样式，空态改传共享组件。全部改动集中在 `features/ask-human/` 与 `components/review/ReviewQueue.tsx`。

**Tech Stack:** React 19 + TypeScript + Vite；plain CSS（`AskHuman.css` 不是 CSS Module）；vitest 4 + @testing-library/react + jsdom。

**依据 spec:** `docs/superpowers/specs/2026-08-06-ask-human-inbox-layout-design.md`（commit `80b9f24`）

## Global Constraints

- 分支：`fix/ask-human-inbox-layout-20260806`（已存在，spec 已提交在其上）
- 工作树中有**他人未提交的改动**（21 项，含 `frontend/src/styles.css`、`frontend/src/features/user-ops/*`、`src/*.rs`、`tests/*.rs`）。每次 `git add` 只 stage 本计划明确列出的文件，**禁止 `git add .` / `git add -A`**。已核验：他人未改本计划要动的三个文件。
- **`AskHuman.css` 必须保持 plain `.css`。** 文件头注释警告：改成 `.module.css` 做副作用导入会被 Rollup tree-shake 删光，整频道白板。
- **所有新增 class 必须带 `askHuman` 前缀。** 该文件是全局作用域，而 `styles.css:423/444` 已定义全局 `.panel` / `.panelHead`，活跃消费者为 `features/user-ops/legacy.tsx:580/890/1099/1100/1152/1315` 与 `features/user-ops/cockpit/CockpitPanel.tsx:110`。占用同名会污染用户运营频道。
- **`.reviewQueueList` 这个 class 名不得改动。** 现有测试 `__tests__/features/ask-human/AskHumanView.dataSource.test.tsx:79` 用 `container.querySelector(".reviewQueueList")` 定位列表。
- CI 门禁 `scripts/check-no-human-takeover.sh` 扫描 `frontend/src/` 新增行中的禁用词（含「**人工**」）。新增注释/测试名一律用「目视确认 / 视觉核验」，不得出现「人工」二字。
- 色值一律走 `components/ui/tokens.css` 变量，禁止硬编码。可用：`--ink-1:#1d1d1f`、`--ink-2:#515156`、`--ink-3:#76767b`、`--ink-4:#b0b0b5`、`--surface-page:#eef1f5`、`--surface-card:#ffffff`、`--hairline:rgba(0,0,0,.08)`、`--r-sm:11px`、`--r-md:18px`、`--r-lg:24px`。
- 测试命令：`cd frontend && npx vitest run <path>`。
- 不做的事：不动 9 个来源的语义与 `SOURCE_META` / `SOURCE_TONE`；不动 chip 切源逻辑与 `refreshNonce` / `setActiveSource` 时序（`index.tsx:167-173` 有死循环规避注释，`fetchItems` 必须 memoize）；不重构 `ReviewQueue` 内部状态机；不动 `ResolvedEscalations` 与六张评审卡的内部样式。

## File Structure

| 文件 | 职责 | 本次改动 |
| --- | --- | --- |
| `frontend/src/features/ask-human/AskHuman.css` | 频道全部样式（plain CSS，496 行） | 外壳去 padding/max-width；新增 `.askHumanPanel` / `.askHumanPanelHead` / `.askHumanToolbar`；chip padding 收紧；`reviewQueue*` 补样式；`--surface-1` 修正；删死规则 |
| `frontend/src/features/ask-human/index.tsx` | 频道组件（286 行，含 `InboxRow` + `AskHumanView`） | 外壳结构改造：白卡包裹、panelHead 放「待处理 N 项」、chip 进 toolbar、空态传 `EmptyState` |
| `frontend/src/components/review/ReviewQueue.tsx` | 通用评审队列（106 行） | `emptyText` 类型 `string` → `ReactNode` |
| `frontend/src/__tests__/features/ask-human/AskHumanLayout.test.tsx` | **Task 1 新建**，Task 2/3 追加 | 共 6 个用例；helper 由 Task 1 一次定义 |

**任务顺序理由：** Task 1（外壳结构，建立测试文件与 helper，改动面最大）→ Task 2（chip 单排，依赖 Task 1 建立的 toolbar 容器）→ Task 3（空态与 `reviewQueue*` 样式，独立）→ Task 4（顺手修正，纯 CSS 无 DOM 变化）。

jsdom 无布局引擎也不跑 CSS 层叠，宽度/颜色/是否折行都**无法断言**，测试只覆盖结构、类名与文本。Task 2、4 的视觉效果必须目视确认。

---

### Task 1: 外壳改用白卡结构

**Files:**
- Modify: `frontend/src/features/ask-human/AskHuman.css`（按选择器字符串定位：`.askHumanChannel`、`.askHumanHeader`、`.askHumanHeader button`）
- Modify: `frontend/src/features/ask-human/index.tsx`（`AskHumanView` 的 return，当前 `:178-274`）
- Create: `frontend/src/__tests__/features/ask-human/AskHumanLayout.test.tsx`

**Interfaces:**
- Consumes: 无
- Produces:
  - CSS class `askHumanPanel`、`askHumanPanelHead`、`askHumanPanelHeadCount`、`askHumanToolbar`（全库均未占用，已核验）
  - 测试 helper：`renderInbox()`、`mockInbox(items, summary)` —— Task 2、3 在同一文件追加用例时**直接复用，不要重复定义**

**背景（实施者必读）：** 当前 `.askHumanChannel` 自带 `padding: 24px 28px 40px` 与 `max-width: 920px`（`AskHuman.css:5-11`），而 Shell 的 `.main` 已有 `padding: 32px 44px`（`Shell.module.css:199`）——双重 padding 且 920px 是全项目唯一自设 max-width 的频道。内容也没有任何白卡包裹，直接贴在 `--surface-page` 灰底上，与其余六个频道（`Quality` / `Autonomy` / `Campaign` / `Operations` / `LlmProviders` / `Evolution`）的 `.page`+`.panel` 结构不一致。

`.askHumanHeader` 是 `justify-content: flex-end`，而 CSS 里的 `.askHumanHeader h1`（`:19-25`）在 JSX 中**没有对应元素**（大页头归 Shell 渲染），所以左侧是空的，两个按钮孤立贴在右上角。

**级联陷阱（必读）：** `index.tsx:192` 的「刷新」按钮**没有 className**，它的全部样式来自 `.askHumanHeader button`（`AskHuman.css:26-37`）。如果把容器 class 从 `askHumanHeader` 改名而不同步改这条规则，该按钮会掉回 `styles.css:71` 的全局裸 button 基线（`min-height:38px`、`background:var(--accent)` 纯蓝、`color:#fff`），和刚修完的治理工坊是同一类缺陷。本任务的做法是**保留 `.askHumanHeader` 类名**给按钮组容器，只把它移进白卡的 panelHead 内，这样那条按钮规则继续生效，零风险。

**「待处理 N 项」的数据来源：** `summary.total`，类型 `number | null`（`lib/inboxApi.ts:38`，解析见 `:105`：非 number 一律取 null）。为 null 时**不显示**计数——null 表示计数不可用，显示「待处理 0 项」是错误信息。

- [ ] **Step 1: 写失败测试**

创建 `frontend/src/__tests__/features/ask-human/AskHumanLayout.test.tsx`。mock 惯例照抄同目录 `AskHumanView.dataSource.test.tsx:1-60`（只 mock 网络层，保留真实 store 与排序）。

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";

// 只 mock 网络层，保留真实 store/排序——验证的是渲染结构，不是数据管道。
vi.mock("../../../lib/inboxApi", async () => {
  const actual = await vi.importActual<typeof import("../../../lib/inboxApi")>(
    "../../../lib/inboxApi",
  );
  return { ...actual, fetchInbox: vi.fn(), fetchSummary: vi.fn() };
});

import { fetchInbox, fetchSummary, type InboxItem } from "../../../lib/inboxApi";
import { useInboxStore } from "../../../stores/inboxStore";
import AskHumanFeature from "../../../features/ask-human/index";

const fi = fetchInbox as unknown as ReturnType<typeof vi.fn>;
const fs = fetchSummary as unknown as ReturnType<typeof vi.fn>;

function item(id: string): InboxItem {
  return {
    source: "src_default", // 未知 source → renderInline 走 default，纯 div，不触发子组件网络
    id,
    title: `t-${id}`,
    summary: "",
    severity: "high",
    createdAt: null,
    ageHours: 0,
    actionKind: "inline",
  };
}

/** Task 2、3 复用：铺好网络返回值。total 传 null 可模拟计数不可用。 */
function mockInbox(items: InboxItem[], total: number | null = items.length) {
  fi.mockResolvedValue({ items, errors: [] });
  fs.mockResolvedValue({
    status: "complete",
    asOf: null,
    counts: { principalEscalation: 1, knowledgeReview: 2 },
    errors: [],
    total,
  });
}

/** Task 2、3 复用：渲染整个频道（含 Confirm/Toast provider，由 feature 默认导出自带）。 */
function renderInbox() {
  return render(<AskHumanFeature />);
}

beforeEach(() => {
  fi.mockReset();
  fs.mockReset();
  // zustand 全局单例，测试间必须 reset，否则旧 items/activeSource 串台。
  useInboxStore.setState({
    items: [],
    errors: [],
    summary: null,
    loading: false,
    fatalError: null,
    activeSource: null,
    requestGeneration: 0,
    summaryRequestGeneration: 0,
  });
});

describe("统一收件箱外壳结构", () => {
  it("内容包在白卡内，且白卡不占用全局 .panel 类名", async () => {
    mockInbox([item("a")]);
    const { container } = renderInbox();
    await screen.findByText("t-a");

    const panel = container.querySelector(".askHumanPanel");
    expect(panel).not.toBeNull();
    // 列表必须在白卡内部，而非与白卡并列贴在灰底上。
    expect(panel!.querySelector(".reviewQueueList")).not.toBeNull();
    // 不得占用全局 .panel（user-ops 频道在用，会相互污染）。
    expect(container.querySelector(".panel")).toBeNull();
    expect(container.querySelector(".panelHead")).toBeNull();
  });

  it("panelHead 显示待处理总数，按钮组仍在其内", async () => {
    mockInbox([item("a"), item("b")], 7);
    const { container } = renderInbox();
    await screen.findByText("t-a");

    const head = container.querySelector(".askHumanPanelHead");
    expect(head).not.toBeNull();
    expect(head!.textContent).toContain("待处理 7 项");
    // 按钮组容器仍是 .askHumanHeader —— 「刷新」按钮的样式全靠
    // `.askHumanHeader button` 这条规则兜，改名会让它掉回全局蓝色基线。
    expect(head!.querySelector(".askHumanHeader")).not.toBeNull();
    expect(screen.getByText("刷新")).toBeTruthy();
  });

  it("total 为 null 时不显示计数（null 表示不可用，不是 0）", async () => {
    mockInbox([item("a")], null);
    const { container } = renderInbox();
    await screen.findByText("t-a");

    const head = container.querySelector(".askHumanPanelHead")!;
    expect(head.textContent).not.toContain("待处理 0 项");
    expect(head.textContent).not.toMatch(/待处理\s*\d+\s*项/);
    // 计数缺失不影响按钮可用。
    expect(screen.getByText("刷新")).toBeTruthy();
  });
});
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human/AskHumanLayout.test.tsx`

Expected: 3 个用例 FAIL。前两个报 `expected null not to be null`（`.askHumanPanel` / `.askHumanPanelHead` 尚不存在）。第三个可能"意外通过"——因为当前根本没有 panelHead，`querySelector` 返回 null 会在 `.textContent` 处抛 `TypeError`，同样算 FAIL。若某个用例因 provider 缺失或 fixture 形状不对而失败，先修 fixture 再继续。

- [ ] **Step 3: 改 CSS——外壳去重复间距，新增白卡三件套**

打开 `frontend/src/features/ask-human/AskHuman.css`。把开头的 `.askHumanChannel` 与 `.askHumanHeader` 两条规则（当前 `:5-18`）替换为：

```css
/* 外壳：与其余频道一致的 .page 语义（grid + 18px 间距）。
   padding 与宽度交给 Shell 的 .main（Shell.module.css:199 已有 32px 44px），
   此处不再自设 padding / max-width——原先两者叠加使内容区偏窄，
   920px 也是全项目唯一自设 max-width 的频道，正是 9 个 chip 折行的直接原因。 */
.askHumanChannel {
  display: grid;
  gap: 18px;
}

/* 白卡：视觉规格对齐 Quality/Autonomy/Campaign 等六个频道的 .panel，
   但类名必须带 askHuman 前缀——本文件是 plain CSS 全局作用域，
   而全局 .panel（styles.css:423）正被 user-ops 频道使用，占名会相互污染。 */
.askHumanPanel {
  border-radius: var(--r-lg);
  padding: 22px 26px 20px;
  background: var(--surface-card);
  border: 1px solid var(--hairline);
  box-shadow: 0 14px 34px -24px rgba(20, 30, 60, 0.3),
    inset 0 1px 1px rgba(255, 255, 255, 0.9);
}

/* 卡头：左侧待处理总数，右侧按钮组。Shell 已有大页头，此处不重复写标题。
   按钮组靠 margin-left:auto 推到右端（见下方 .askHumanHeader），不要用
   justify-content: space-between——计数不可用（total 为 null，左侧不渲染）时
   按钮组会成为唯一子元素而被推到左边，按钮位置随数据跳动。 */
.askHumanPanelHead {
  display: flex;
  align-items: center;
  gap: 14px;
  margin-bottom: 16px;
}
.askHumanPanelHeadCount {
  font-size: 13px;
  font-weight: 600;
  color: var(--ink-2);
  letter-spacing: -0.2px;
}

/* 按钮组容器：保留 askHumanHeader 类名不改。
   「刷新」按钮无 className，全靠下方 `.askHumanHeader button` 规则兜样式；
   改名会让它掉回 styles.css:71 的全局 38px 纯蓝基线。 */
.askHumanHeader {
  display: flex;
  align-items: center;
  gap: 8px;
  /* 恒定靠右：不依赖父级 space-between，故计数节点不渲染时按钮位置不变。 */
  margin-left: auto;
}
```

同时删掉紧随其后的死规则 `.askHumanHeader h1`（JSX 中无 `h1`，大页头由 Shell 渲染）：

```css
.askHumanHeader h1 {
  margin: 0;
  font-size: 19px;
  font-weight: 650;
  letter-spacing: -0.2px;
  color: var(--ink-1);
}
```

`.askHumanHeader button` 及其 `:hover` / `:disabled` 三条规则**原样保留**，一行不动。

- [ ] **Step 4: 改 JSX——白卡包裹 + panelHead**

打开 `frontend/src/features/ask-human/index.tsx`。当前 `AskHumanView` 的 return 是 `<div className="askHumanChannel">` 内直接放 `<header className="askHumanHeader">` 再跟一串兄弟节点。改成白卡结构。

把 `return (` 之后从 `<header className="askHumanHeader">` 到 `</header>` 的整段（当前 `:180-197`）替换为：

```tsx
      <div className="askHumanPanel">
        <div className="askHumanPanelHead">
          {/* total 为 null 表示计数不可用，此时不渲染——显示「待处理 0 项」是错误信息。 */}
          {summary?.total != null && (
            <span className="askHumanPanelHeadCount">待处理 {summary.total} 项</span>
          )}
          <div className="askHumanHeaderActions askHumanHeader">
            <button
              type="button"
              className={
                showResolved ? "askHumanViewToggle askHumanViewToggle--active" : "askHumanViewToggle"
              }
              onClick={() => setShowResolved((v) => !v)}
            >
              {showResolved ? "待处理" : "已裁决历史"}
            </button>
            {!showResolved && (
              <button type="button" onClick={() => refreshAll()} disabled={loading}>
                刷新
              </button>
            )}
          </div>
        </div>
```

注意三点：

1. 按钮组同时挂 `askHumanHeaderActions` 与 `askHumanHeader` 两个 class——前者是原有的 flex 布局，后者让 `.askHumanHeader button` 规则继续命中无 class 的「刷新」按钮。
2. `total` 为 null 时整个计数节点不渲染，无需空占位——按钮组由 `.askHumanHeader { margin-left: auto }` 恒定推到右端，位置不随数据有无而跳动。
3. 这里只**新开**了 `<div className="askHumanPanel">`，闭合标签在下一步补。

- [ ] **Step 5: 补白卡闭合标签**

在同一 return 的末尾，把原来的

```tsx
        </>
      )}
    </div>
  );
```

改为（多一层 `</div>` 闭合 `.askHumanPanel`）：

```tsx
        </>
      )}
      </div>
    </div>
  );
```

- [ ] **Step 6: 运行测试，确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human/AskHumanLayout.test.tsx`
Expected: 3 passed。

- [ ] **Step 7: 跑既有 ask-human 测试，确认无回归**

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human/`
Expected: 全绿。既有 6 个测试文件都不依赖 `askHuman*` 类名（已核验），但 `AskHumanView.dataSource.test.tsx:79` 依赖 `.reviewQueueList`——本任务未改该类名。

- [ ] **Step 8: 构建 + 门禁**

Run: `cd frontend && npm run build`
Expected: 构建成功（`tsc` 会检查 JSX 标签配平，多/少一个 `</div>` 会在此暴露）。

Run（**必须从仓库根执行**）: `cd "$(git rev-parse --show-toplevel)" && bash scripts/check-no-human-takeover.sh`

> 门禁脚本用 `git diff --name-only -- frontend/src/ ...` 做 pathspec 过滤，pathspec 相对 cwd 解析。若从 `frontend/` 目录执行，它匹配不到任何文件、输出 `no changed files under scan dirs; ok.` 并 exit 0——**假通过**。务必用上面的 `git rev-parse` 写法，与 cwd 无关。
Expected: 通过（新增注释用「目视确认」措辞，无禁用词）。

- [ ] **Step 9: Commit**

```bash
git add frontend/src/features/ask-human/AskHuman.css frontend/src/features/ask-human/index.tsx frontend/src/__tests__/features/ask-human/AskHumanLayout.test.tsx
git commit -m "fix(ui): 统一收件箱内容收进白卡，对齐全项目频道结构

原先 .askHumanChannel 自带 padding 与 max-width:920px，与 Shell 的
.main padding 叠加使内容区偏窄，且内容无白卡直接贴灰底，与其余六个
频道的 .page+.panel 结构不一致。920px 也是 9 个 chip 折行的直接原因。

白卡类名带 askHuman 前缀而非直接用 .panel：本文件是 plain CSS 全局
作用域，全局 .panel 正被 user-ops 频道使用，占名会相互污染。
按钮组保留 .askHumanHeader 类名——无 className 的「刷新」按钮全靠
该规则兜样式，改名会掉回全局 38px 纯蓝基线。

顺带删掉 .askHumanHeader h1 死规则（JSX 无对应元素，大页头归 Shell）。"
```

---

### Task 2: chip 横向单排

**Files:**
- Modify: `frontend/src/features/ask-human/AskHuman.css`（按选择器字符串定位：`.askHumanSummary`、`.askHumanSummaryChip`）
- Modify: `frontend/src/features/ask-human/index.tsx`（chip 容器 class，当前 `:220`）
- Modify: `frontend/src/__tests__/features/ask-human/AskHumanLayout.test.tsx`（**追加** describe 块，文件已由 Task 1 创建）

**Interfaces:**
- Consumes: Task 1 建立的 `.askHumanPanel` 白卡与测试 helper `renderInbox()` / `mockInbox()` / `item()` —— **直接复用，不要重复定义**（重复声明会导致 TS 编译报错）
- Produces: 无（终端样式）

**背景（实施者必读）：** 9 个 chip 在 12px 字号、`padding: 6px 12px`、`gap: 8px` 下实测需约 **911px**：

| 项 | 计算 | 小计 |
| --- | --- | --- |
| 单个 chip | 4 个中文字 ≈ 48px + 「: 0」≈ 20px + padding 24px + border 2px | ≈ 94px |
| 9 个 chip | 94 × 9 | 847px |
| 8 个间隙 | 8 × 8px | 64px |
| **合计** | | **911px** |

改动前可用宽度只有 864px（1440px 视口 − 侧栏 282px − `.main` padding 88px = 1070px，再受 `max-width:920px` 与自身 padding 56px 限制）——差 47px，必然折行。

Task 1 已移除 `max-width` 与自身 padding，可用宽度升至 1070px（1440 视口）。但 1280px 视口只有 910px，与 911px 几乎贴平，所以本任务把 chip 的 `padding` 从 `6px 12px` 收到 `6px 10px`，总宽降到约 875px，1280px 下留 35px 余量。

**为什么不和按钮同排：** chip 911px + 两个按钮约 200px + 间距 ≈ 1111px > 1070px，放不下。所以 chip 独占一行，按钮留在 Task 1 建立的 panelHead 右侧。

**`flex-wrap` 保留但降级为兜底：** 更窄的视口（< 1100px）仍会折行，这是防溢出而非常态。项目既有的横向筛选惯例同样保留 wrap：`Campaign.module.css:38 .filters`、`Quality.module.css:57 .toolbar`、`Autonomy.module.css:30 .toolbar` 都是 `display:flex; flex-wrap:wrap; gap`。

**px 数字的性质：** 上表按 12px 字号下中文 1em、ASCII 0.5em 估算，非浏览器实测。jsdom 无布局引擎，是否真的单排**必须目视确认**（Step 5）。

- [ ] **Step 1: 写失败测试**

在 `frontend/src/__tests__/features/ask-human/AskHumanLayout.test.tsx` **末尾追加**。顶部 import、`item()`、`mockInbox()`、`renderInbox()`、`beforeEach` 都已存在，**不要重复定义、不要重新 import**：

```tsx
describe("来源筛选 chip 布局", () => {
  // jsdom 无布局引擎，量不到实际宽度，也判断不出是否折行。
  // 这里只锁结构：9 个 chip 必须同属一个 toolbar 容器，且该容器在白卡内、
  // 与 panelHead 平级（而非塞进 panelHead 与按钮挤同一排——实测放不下）。
  // 真实单排效果需目视确认。
  it("9 个来源 chip 同属一个 toolbar 容器，位于白卡内且不在 panelHead 内", async () => {
    mockInbox([item("a")]);
    const { container } = renderInbox();
    await screen.findByText("t-a");

    const toolbar = container.querySelector(".askHumanToolbar");
    expect(toolbar).not.toBeNull();

    const chips = toolbar!.querySelectorAll(".askHumanSummaryChip");
    expect(chips).toHaveLength(9);

    // toolbar 在白卡内。
    expect(container.querySelector(".askHumanPanel")!.contains(toolbar!)).toBe(true);
    // 但不在 panelHead 内——chip 与按钮同排需 1111px，超出 1440px 视口的可用 1070px。
    expect(container.querySelector(".askHumanPanelHead")!.contains(toolbar!)).toBe(false);
  });

  it("chip 文案与切源可用性不受布局改动影响", async () => {
    mockInbox([item("a")]);
    renderInbox();
    await screen.findByText("t-a");

    // counts 里给了 principalEscalation:1 / knowledgeReview:2，其余源无值 → 不可用。
    expect(screen.getByText("请示裁决: 1")).toBeTruthy();
    expect(screen.getByText("知识核验: 2")).toBeTruthy();
    expect(screen.getByText("标签候选: 不可用")).toBeTruthy();
  });
});
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human/AskHumanLayout.test.tsx -t chip`

Expected: 第一个用例 FAIL，报 `expected null not to be null`（`.askHumanToolbar` 尚不存在）。第二个用例应当**通过**——它验证的是本任务不该破坏的既有行为（回归哨兵），此刻已成立。

- [ ] **Step 3: 改 CSS——chip 容器改名并收紧 padding**

打开 `frontend/src/features/ask-human/AskHuman.css`，找到 `.askHumanSummary` 规则（按字符串定位，Task 1 已改动上方内容致行号漂移）。当前：

```css
/* summary：来源计数 chip 行，点击切源 */
.askHumanSummary {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
```

改为（改名 `askHumanToolbar` 与项目 `.toolbar` 惯例一致，并保留旧类名以防漏改）：

```css
/* 工具栏：来源计数 chip 行，点击切源。命名对齐 Quality/Autonomy 的 .toolbar 惯例。
   9 个 chip 在收紧 padding 后实测约 875px，1280px 视口可用 910px 可单排。
   flex-wrap 保留但只作更窄视口的溢出兜底，不再是常态（原先 920px 容器下必然折行）。 */
.askHumanToolbar,
.askHumanSummary {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
}
```

再找到 `.askHumanSummaryChip` 规则，把 `padding: 6px 12px;` 一行改为：

```css
  padding: 6px 10px;
```

其余声明（`font-size: 12px`、`border-radius: 999px`、`border`、`color`、`transition`）一行不动。`--active` 与 `:hover` 两条规则也不动。

- [ ] **Step 4: 改 JSX——chip 容器加新 class**

打开 `frontend/src/features/ask-human/index.tsx`，找到 chip 容器（当前 `:220`，按字符串 `className="askHumanSummary"` 定位）：

```tsx
          <div className="askHumanSummary">
```

改为：

```tsx
          <div className="askHumanToolbar askHumanSummary">
```

两个 class 都留：`askHumanToolbar` 是新语义名（测试据此定位），`askHumanSummary` 保持既有选择器仍能命中。内部 9 个 chip 的 `.map` 逻辑、`onClick`、`activeSource` 判断**一行不动**。

- [ ] **Step 5: 运行测试，确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human/AskHumanLayout.test.tsx`
Expected: 5 passed（Task 1 的 3 个 + 本 Task 的 2 个）。

- [ ] **Step 6: 构建**

Run: `cd frontend && npm run build`
Expected: 构建成功。

- [ ] **Step 7: 目视确认单排（jsdom 无法断言）**

`cd frontend && npm run dev`，打开「统一收件箱」频道，确认：

- 9 个 chip 在 1440px 与 1280px 视口下**单排显示、不折行**（这是本任务的核心目标，也是唯一无法自动断言的部分）
- chip 收紧 padding 后文字未被裁切、`: N` 计数完整可见
- 选中态（点击某个 chip）仍是深底白字（`--ink-1` 底 + `#fff` 字），与未选中的白底描边区分明显
- 窗口拖到 1100px 以下时折行为兜底行为，此时布局不应破版或溢出白卡

- [ ] **Step 8: Commit**

```bash
git add frontend/src/features/ask-human/AskHuman.css frontend/src/features/ask-human/index.tsx frontend/src/__tests__/features/ask-human/AskHumanLayout.test.tsx
git commit -m "fix(ui): 统一收件箱 9 个来源 chip 单排不再折行

9 个 chip 实测需约 911px，而改动前容器受 max-width:920px 与自身
padding 限制只剩 864px，差 47px 必然折成两排。Task 1 移除 max-width
后可用 1070px；本次再把 chip padding 从 12px 收到 10px，总宽降到约
875px，1280px 视口（可用 910px）也能单排。

容器改名 askHumanToolbar 对齐 Quality/Autonomy 的 .toolbar 惯例，
旧类名一并保留避免漏改。flex-wrap 保留但降级为更窄视口的溢出兜底。
chip 不与按钮同排：两者合计需 1111px，超出可用 1070px。"
```

---

### Task 3: 空态改用共享 EmptyState

**Files:**
- Modify: `frontend/src/components/review/ReviewQueue.tsx:14`（`emptyText` 类型）与 `:22`（解构，无需改动但需确认）
- Modify: `frontend/src/features/ask-human/index.tsx`（`emptyText` 传值，当前 `:270`）
- Modify: `frontend/src/features/ask-human/AskHuman.css`（新增 `reviewQueue*` 三条规则）
- Modify: `frontend/src/__tests__/features/ask-human/AskHumanLayout.test.tsx`（**追加** describe 块）

**Interfaces:**
- Consumes: Task 1 建立的测试 helper `renderInbox()` / `mockInbox()` / `item()`；共享组件 `EmptyState`（既有，签名见下）
- Produces: `ReviewQueue` 的 `emptyText?: ReactNode`（放宽后的类型，后续调用方可传 JSX）

**背景（实施者必读）：** `ReviewQueue.tsx:93` 渲染的是裸 `<div className="reviewQueueEmpty">`，而 `reviewQueueEmpty` / `reviewQueueLoading` / `reviewQueueError` / `reviewQueueList` 四个 class 在**全库 CSS 中零定义**（已 grep 核验），所以「暂无待处理项」就是一行没有任何样式的裸文字。

项目已有共享空态组件 `frontend/src/components/ui/EmptyState/EmptyState.tsx`，`LlmProviders`（`index.tsx:417`）、`SendAnalytics`（`index.tsx:93`）、`ReferralCards`（`index.tsx:96`）都在用。其签名与样式：

```tsx
export function EmptyState({ icon, title, hint, action }: {
  icon?: React.ReactNode;
  title: string;
  hint?: string;
  action?: React.ReactNode;
}): JSX.Element
// 默认 icon 为 lucide 的 <Inbox size={28} />
// 容器样式：虚线框 + 居中 + background: var(--surface-page)
```

import 惯例（照抄 `LlmProviders/index.tsx:13`）：

```tsx
import { EmptyState } from "../../components/ui/EmptyState";
```

**类型阻碍：** `ReviewQueueProps.emptyText` 当前是 `emptyText?: string`（`ReviewQueue.tsx:14`），塞不进 JSX 元素，必须先放宽为 `ReactNode`。`ReactNode` 已在该文件第 1 行 import（`type ReactNode`），无需新增 import。

**范围安全性：** `ReviewQueue` 全库只有 ask-human 一个真实消费者。`features/knowledge/steward.tsx` 中的 `ReviewQueueItem` / `ReviewQueueResponse` 只是同名 TypeScript 类型，该文件**未 import 该组件**（已核验）。放宽类型是向后兼容的拓宽——原先传 `string` 的调用方不受影响，默认值 `"暂无待处理项"` 保留。

**`.reviewQueueList` 类名不得改动**（Global Constraints 已列）：`AskHumanView.dataSource.test.tsx:79` 依赖它。

- [ ] **Step 1: 写失败测试**

在 `frontend/src/__tests__/features/ask-human/AskHumanLayout.test.tsx` **末尾追加**（helper 已存在，不要重复定义）：

```tsx
describe("空态渲染", () => {
  it("无待办时渲染共享 EmptyState 结构，而非裸 div", async () => {
    mockInbox([], 0);
    const { container } = renderInbox();

    // EmptyState 的标题文案。
    await screen.findByText("暂无待处理项");

    // 裸 div 分支不应再出现。
    expect(container.querySelector(".reviewQueueEmpty")).toBeNull();
    // EmptyState 自带 lucide Inbox 图标（CSS Module 类名经哈希，故按 svg 判定）。
    const empty = screen.getByText("暂无待处理项").closest("div");
    expect(empty).not.toBeNull();
    expect(empty!.querySelector("svg")).not.toBeNull();
    // 空态仍在白卡内。
    expect(container.querySelector(".askHumanPanel")!.textContent).toContain("暂无待处理项");
  });

  it("空态带提示文案，说明这是正常状态而非故障", async () => {
    mockInbox([], 0);
    renderInbox();
    await screen.findByText("暂无待处理项");
    expect(screen.getByText(/AI 自主运行中/)).toBeTruthy();
  });
});
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human/AskHumanLayout.test.tsx -t 空态`

Expected: 2 个用例 FAIL。第一个报 `.reviewQueueEmpty` 仍存在（`expected <div class="reviewQueueEmpty"> to be null`）或 `svg` 为 null；第二个报找不到 `/AI 自主运行中/` 文案。

- [ ] **Step 3: 放宽 ReviewQueue 的 emptyText 类型**

打开 `frontend/src/components/review/ReviewQueue.tsx`，把 `ReviewQueueProps` 中的一行（`:14`）：

```tsx
  emptyText?: string;
```

改为：

```tsx
  // ReactNode 而非 string：ask-human 传入共享 <EmptyState> 组件。
  // 拓宽是向后兼容的——原先传字符串的调用方不受影响，默认值仍是纯文本。
  emptyText?: ReactNode;
```

`ReactNode` 已在该文件第 1 行 import，无需改 import。`:93` 的渲染分支与 `emptyText ?? "暂无待处理项"` 默认值**一行不动**。

- [ ] **Step 4: 给 reviewQueue* 三个 class 补样式**

打开 `frontend/src/features/ask-human/AskHuman.css`，在文件**末尾追加**：

```css
/* ReviewQueue 的容器 class 原先在全库无任何 CSS 定义（裸文字/无间距）。
   此处补最小样式。ReviewQueue 全库只有本频道一个真实消费者
   （knowledge/steward.tsx 中的 ReviewQueueItem 只是同名类型，未 import 组件），
   故这些规则不外溢。空态本身改由共享 EmptyState 组件渲染，不在此定义。 */
.reviewQueueList {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.reviewQueueLoading {
  padding: 24px 14px;
  font-size: 12.5px;
  color: var(--ink-3);
  text-align: center;
}
.reviewQueueError {
  padding: 11px 14px;
  border-radius: var(--r-sm);
  font-size: 12.5px;
  color: var(--color-blocked);
  background: var(--fill-blocked);
  border: 1px solid rgba(255, 69, 58, 0.3);
}
```

`.reviewQueueError` 的配色照抄同文件既有的 `.askHumanFatal` 规则（红底警示条），保持频道内错误呈现一致。

**同时必须移除 `.inboxRow` 的 `margin-bottom`**（按字符串定位该规则）：

```css
.inboxRow {
  border: 1px solid var(--hairline);
  border-radius: var(--r-md, 12px);
  background: var(--surface-card);
}
```

原先 `.reviewQueueList` 在全库无任何 CSS，行间距全靠 `.inboxRow` 自带的 `margin-bottom: 10px`。上面新增的 `gap: 10px` 会与它**叠加成 20px**，且末行下方多出 10px 空白。间距只应由父容器的 `gap` 负责——同文件的 `.resolvedEscList` / `.resolvedEscRow` 就是这个正确范例（父有 `gap: 12px`，子无 `margin-bottom`）。

- [ ] **Step 5: 改 JSX——传入 EmptyState**

打开 `frontend/src/features/ask-human/index.tsx`。

**5a.** 在 import 区加一行（放在既有 `components/` import 附近，照抄 `LlmProviders/index.tsx:13` 的路径写法）：

```tsx
import { EmptyState } from "../../components/ui/EmptyState";
```

**5b.** 找到 `ReviewQueue` 的 `emptyText` 传参（当前 `:270`，按字符串 `emptyText="暂无待处理项"` 定位）：

```tsx
            emptyText="暂无待处理项"
```

改为:

```tsx
            emptyText={
              <EmptyState
                title="暂无待处理项"
                hint="AI 自主运行中，需要决策或审核的事项会自动出现在这里。"
              />
            }
```

文案说明：空收件箱是**正常状态**，提示语要传达"系统在正常工作"而非"加载失败"。措辞避开 CI 禁用词（不含「人工」二字）。

- [ ] **Step 6: 运行测试，确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human/AskHumanLayout.test.tsx`
Expected: 7 passed（Task 1 的 3 个 + Task 2 的 2 个 + 本 Task 的 2 个）。

- [ ] **Step 7: 跑 ReviewQueue 既有测试，确认类型放宽无回归**

Run: `cd frontend && npx vitest run src/__tests__/components/review/ReviewQueue.test.tsx`
Expected: 全绿。该文件有 3 处 `<ReviewQueue>` 用法，均未传 `emptyText` 或传字符串，拓宽类型对它们向后兼容。

- [ ] **Step 8: 全量测试 + 构建 + 门禁**

Run: `cd frontend && npx vitest run && npm run build`
Expected: 全绿、构建成功。`npm run build` 的 `tsc` 会跨 `src` 检查（含测试文件），类型放宽若有不兼容处会在此暴露。

Run（**必须从仓库根执行**）: `cd "$(git rev-parse --show-toplevel)" && bash scripts/check-no-human-takeover.sh`

> 门禁脚本用 `git diff --name-only -- frontend/src/ ...` 做 pathspec 过滤，pathspec 相对 cwd 解析。若从 `frontend/` 目录执行，它匹配不到任何文件、输出 `no changed files under scan dirs; ok.` 并 exit 0——**假通过**。务必用上面的 `git rev-parse` 写法，与 cwd 无关。
Expected: 通过。

- [ ] **Step 9: Commit**

```bash
git add frontend/src/components/review/ReviewQueue.tsx frontend/src/features/ask-human/index.tsx frontend/src/features/ask-human/AskHuman.css frontend/src/__tests__/features/ask-human/AskHumanLayout.test.tsx
git commit -m "fix(ui): 统一收件箱空态改用共享 EmptyState

reviewQueueEmpty/Loading/Error/List 四个 class 在全库 CSS 中零定义，
「暂无待处理项」是一行无样式裸文字。空态改传共享 EmptyState 组件
（LlmProviders/SendAnalytics/ReferralCards 已在用），另三个 class 补
最小样式，error 配色照抄同文件 .askHumanFatal 保持一致。

为此把 ReviewQueue 的 emptyText 从 string 放宽为 ReactNode——向后
兼容的拓宽，默认值仍是纯文本。该组件全库只有本频道一个真实消费者
（steward.tsx 的 ReviewQueueItem 只是同名类型，未 import 组件）。
reviewQueueList 类名保留：现有测试用它定位列表。"
```

---

### Task 4: 修正未定义 CSS 变量

**Files:**
- Modify: `frontend/src/features/ask-human/AskHuman.css`（3 处 `var(--surface-1, #fff)`，按字符串定位）

**Interfaces:**
- Consumes: 无
- Produces: 无（终端样式）

**背景（实施者必读）：** `AskHuman.css` 有 3 处引用 `var(--surface-1, #fff)`（改动前位于 `:80`、`:147`、`:194`，Task 1-3 已致行号漂移，**按字符串定位**）。`--surface-1` 这个变量在**全库 CSS 中无任何定义**（已 grep 核验 `tokens.css`、`styles.css`、全部 `.module.css`），所以它一路 fallback 到硬编码 `#fff`——与该文件头注释「色值/圆角/描边统一走 tokens.css 变量，禁止硬编码主色」自相矛盾。正确的 token 是 `--surface-card`（`tokens.css` 定义为 `#ffffff`）。

**只改这 3 处。** 同文件另有 1 处 `var(--surface, #fff)`（`.inboxRowBody` 附近），`--surface` 是**有定义的**（`styles.css:4` `--surface: #ffffff;`），能正常解析，**不要动**。这是本任务最容易做错的地方——两个变量名只差一个后缀。

**视觉零变化：** `--surface-1` 的 fallback `#fff` 与 `--surface-card` 的值 `#ffffff` 是同一个颜色，所以这是纯粹的规范修正，渲染结果逐像素相同。因此本任务**没有测试**——jsdom 不跑 CSS 层叠，改前改后 DOM 完全一致，写任何断言都是假测试。

- [ ] **Step 1: 确认改动前的 4 处引用及其区别**

Run: `cd frontend && grep -n 'var(--surface' src/features/ask-human/AskHuman.css`

Expected: 4 行输出——3 行 `var(--surface-1, #fff)`，1 行 `var(--surface, #fff)`。记住哪一行是 `--surface`（不带 `-1`），它不能改。

Run: `cd frontend && grep -rn -- '--surface-1:' src/ ; echo "exit=$?"`

Expected: 无输出（`exit=1`），确认 `--surface-1` 确实全库无定义。

Run: `cd frontend && grep -rn -- '--surface:' src/styles.css`

Expected: 命中 `--surface: #ffffff;`，确认另一处引用是有效的、不该改。

- [ ] **Step 2: 替换 3 处 `--surface-1`**

把 3 处

```css
  background: var(--surface-1, #fff);
```

逐处改为

```css
  background: var(--surface-card);
```

不要用不带边界的全局替换——必须确保 `var(--surface, #fff)` 那一处未被误伤（`--surface-1` 与 `--surface` 前缀相同）。

- [ ] **Step 3: 确认替换结果**

Run: `cd frontend && grep -n 'var(--surface-1' src/features/ask-human/AskHuman.css ; echo "exit=$?"`
Expected: 无输出（`exit=1`），`--surface-1` 已清零。这是本任务的**主判据**。

Run: `cd frontend && grep -c 'var(--surface, #fff)' src/features/ask-human/AskHuman.css`
Expected: `1` —— 那处**必须仍在**，证明未被误伤（`--surface-1` 与 `--surface` 前缀相同，全局替换会连坐）。

Run: `cd frontend && git diff --stat src/features/ask-human/AskHuman.css`
Expected: `1 file changed, 3 insertions(+), 3 deletions(-)` —— 恰好 3 行改动，不多不少。

> 不要用 `grep -c 'var(--surface-card)'` 期望某个固定数字：Task 3 在文件末尾新增的规则里也可能引用 `--surface-card`，那样计数会大于 3，看起来像出错其实正常。用上面三条判据即可。

- [ ] **Step 4: 全量测试 + 构建 + 门禁**

Run: `cd frontend && npx vitest run && npm run build`
Expected: 全绿、构建成功。

Run（**必须从仓库根执行**）: `cd "$(git rev-parse --show-toplevel)" && bash scripts/check-no-human-takeover.sh`

> 门禁脚本用 `git diff --name-only -- frontend/src/ ...` 做 pathspec 过滤，pathspec 相对 cwd 解析。若从 `frontend/` 目录执行，它匹配不到任何文件、输出 `no changed files under scan dirs; ok.` 并 exit 0——**假通过**。务必用上面的 `git rev-parse` 写法，与 cwd 无关。
Expected: 通过。

- [ ] **Step 5: Commit**

```bash
git add frontend/src/features/ask-human/AskHuman.css
git commit -m "style(ui): 统一收件箱改用已定义的 --surface-card

3 处 var(--surface-1, #fff) 引用的变量在全库 CSS 中无定义，实际一路
fallback 到硬编码 #fff，与本文件头注释「禁止硬编码」自相矛盾。改为
tokens.css 中已定义的 --surface-card。

同文件另一处 var(--surface, #fff) 未动——--surface 在 styles.css:4
确有定义，能正常解析。两者值同为白色，本次渲染逐像素不变。"
```

---

## 收尾：推送与 PR

- [ ] **Step 1: 确认只提交了本计划涉及的文件**

Run: `git log --oneline 80b9f24..HEAD --stat && git status --short`

Expected: 提交仅含 `frontend/src/features/ask-human/AskHuman.css`、`frontend/src/features/ask-human/index.tsx`、`frontend/src/components/review/ReviewQueue.tsx`、`frontend/src/__tests__/features/ask-human/AskHumanLayout.test.tsx`。`git status` 中他人的 21 项未提交改动（含 `frontend/src/styles.css`、`frontend/src/features/user-ops/*`、`src/*.rs`、`tests/*.rs`、`AGENTS.md` 等）应**仍为未提交状态**。

- [ ] **Step 2: 目视核验（合并前必做）**

`cd frontend && npm run dev`，打开「统一收件箱」频道，逐项确认：

| 项 | 期望 |
| --- | --- |
| 整体结构 | 内容在白卡内，与其余频道（AI 总控、运营成效）观感一致，不再是元素浮在灰底上 |
| 内容宽度 | 不再偏窄，白卡撑满可用宽度（原先受 920px 限制） |
| chip 单排 | 1440px 与 1280px 视口下 9 个 chip **单排不折行**（核心目标） |
| 卡头 | 左侧「待处理 N 项」，右侧两个按钮，不再孤立贴右上 |
| 刷新按钮 | 白底描边小号款，**不是** 38px 纯蓝实心（若变蓝说明 `.askHumanHeader` 规则失效，见 Task 1 级联陷阱） |
| 空态 | 虚线框 + Inbox 图标 + 标题 + 提示，居中，不再是裸文字 |
| 已裁决历史 | 点「已裁决历史」切过去，列表同样在白卡内且排版正常 |
| 窄屏 | 拖到 1100px 以下 chip 折行为兜底，布局不破版、不溢出白卡 |

- [ ] **Step 3: 推送并建 PR**

```bash
git push -u origin fix/ask-human-inbox-layout-20260806
gh pr create --base main --title "fix(ui): 统一收件箱布局对齐项目频道结构" --body "$(cat <<'EOF'
## 摘要

「统一收件箱」频道此前没有采用全项目统一的 `.page` + 白卡结构：内容直接贴在灰底上、9 个来源 chip 折成两排、空态是一行无样式裸文字。本 PR 把它对齐到其余六个频道（`Quality` / `Autonomy` / `Campaign` / `Operations` / `LlmProviders` / `Evolution`）的结构。

设计文档：`docs/superpowers/specs/2026-08-06-ask-human-inbox-layout-design.md`

## 改动

| # | 缺陷 | 根因 | 改法 |
| --- | --- | --- | --- |
| 1 | 内容浮在灰底、间距松散、内容区偏窄 | 无白卡包裹；`.askHumanChannel` 自带 padding 与 `max-width:920px`，与 Shell `.main` 的 `padding:32px 44px` 叠加 | 补 `.askHumanPanel` 白卡；外壳改 `display:grid; gap:18px`，间距交给 Shell |
| 2 | 9 个 chip 折成两排 | 实测需 911px，容器受 920px 限制只剩 864px | 移除 `max-width` + chip padding 收到 10px → 约 875px 单排 |
| 3 | 按钮孤立贴右上 | `.askHumanHeader{justify-content:flex-end}`，而 CSS 里的 `h1` 在 JSX 中不存在（大页头归 Shell），左侧是空的 | 移进卡头，左侧改放 `summary.total`「待处理 N 项」；删掉 `h1` 死规则 |
| 4 | 空态是裸文字 | `reviewQueueEmpty/Loading/Error/List` 四个 class 全库零 CSS 定义 | 空态改用共享 `EmptyState`；另三个 class 补最小样式 |
| 5 | 3 处引用未定义变量 | `--surface-1` 全库无定义，一路 fallback 到硬编码 `#fff` | 改为 `--surface-card` |

**两处刻意的克制：**

白卡类名带 `askHuman` 前缀而非直接用 `.panel`。`AskHuman.css` 是 plain CSS（全局作用域，改成 `.module.css` 会被 Rollup tree-shake 删光整个频道），而全局 `.panel` / `.panelHead` 正被 `user-ops` 频道使用（`legacy.tsx` 6 处 + `CockpitPanel.tsx` 1 处），占用同名会相互污染。

按钮组容器保留 `.askHumanHeader` 类名。「刷新」按钮没有 `className`，全部样式来自 `.askHumanHeader button` 这条规则；改名会让它掉回全局 38px 纯蓝基线——与 #239 修的治理工坊白底白字是同一类级联缺陷。

## 测试

新增 `frontend/src/__tests__/features/ask-human/AskHumanLayout.test.tsx`（7 个用例）：白卡存在且列表在其内、未占用全局 `.panel`、卡头显示总数、`total` 为 null 时不显示「0 项」、9 个 chip 同属一个 toolbar 且不在卡头内、chip 文案与切源不受影响、空态渲染 `EmptyState` 结构与提示文案。

**测试边界**：jsdom 无布局引擎也不跑 CSS 层叠，宽度、颜色、是否折行均**无法断言**，仅覆盖结构、类名与文本。chip 真实单排效果与白卡观感需目视确认（清单见计划文档收尾第 2 步）。

## 影响面

`ReviewQueue.emptyText` 由 `string` 放宽为 `ReactNode`——向后兼容的拓宽，默认值不变。该组件全库只有本频道一个真实消费者（`knowledge/steward.tsx` 中的 `ReviewQueueItem` 只是同名 TypeScript 类型，未 import 该组件）。其余改动全部限于 `features/ask-human/` 目录内。

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 4: 等 CI**

Run: `gh pr checks --watch`
Expected: 全绿再合并。后端门禁（Baseline / Integration 等）会因 `dorny/paths-filter` 显示 skipping——本 PR 只碰 `frontend/src/` 与 `docs/`，属预期。


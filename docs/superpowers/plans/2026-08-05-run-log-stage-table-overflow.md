# 运行日志展开行阶段表格溢出 — 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修掉「运营 → 任务、事件与复核 → 运行日志」展开行里阶段 key-value 表格横向溢出屏幕、阶段标签被挤成逐字竖排的问题，并顺带补齐 `thead`/`td` 列数不一致与长 JSON 值无断点两处隐患。

**Architecture:** 根因是**类名误用**而非 CSS 写错。`.tHead`（`Operations.module.css:83`，`display:flex` + `space-between`）是为事件时间线设计的——`<strong>标签</strong>` + `<span>时间</span>` 左右分列。但 `index.tsx:522` 把它复用到阶段区块，里面塞的是 `<strong>` + 一个 `width:100%` 的嵌套 `<table>`；flex 项默认 `min-width:auto` 不肯收缩到内容宽度以下，表格向外撑、`<strong>` 被压到最小宽度。修法是给阶段区块**新建** `.stageBlock`/`.stageTable` 两个类（纵向堆叠 + `table-layout:fixed` + `word-break`），**不动 `.tHead`**——事件时间线在正确使用它。再加一行全局守卫 `.main { min-width: 0 }`（`Shell.module.css:195`）阻止任何页面的宽内容撑破整页。

**Tech Stack:** React 19 + TypeScript + CSS Modules + Vite。测试 Vitest + @testing-library/react（jsdom）。校验 `npx tsc --noEmit`、`npx vitest run`、`npm run build`。

## Global Constraints

- **不改 `.tHead`**：事件时间线（`index.tsx:308`）正确使用它（`<strong>` + `<span>` 左右分列），改它会连带破坏。阶段区块必须换用新类。
- **不重构 `renderStageValue` 的数据形态**：不改成折叠 JSON 树/语法高亮，只让长串能折行。超出本 bug 范围。
- **不动其他 feature 的表格**：`knowledge`、`ask-human` 等页面的表格不在本次范围。
- **jsdom 不做布局**：CSS Modules 在 vitest 里只产出类名哈希、不注入真实样式，`getComputedStyle` 拿不到 `display:flex`。故**可自动化的只有结构断言**（类名、DOM 层级、`th`/`td` 计数）；CSS 属性值靠 CSS 文件本身的断言 + 部署后目视复核。计划里每个任务都标明它属于哪一类。
- **既有 4 个 runs 测试不得回归**：`operations.test.tsx` 里 `describe("运行日志 runs tab + tier 遥测(C6+C9)")` 的 4 条必须继续通过。
- **基线不回归**：`npx vitest run` 全量 618 tests / 125 files 全绿；`npx tsc --noEmit` 无错；`npm run build` 成功。
- **前端 lint 红线**：新增行禁 `人工/接管/托管/takeover/hand-off`（`scripts/check-no-human-takeover.sh` 扫 `frontend/src/`）。

---

## File Structure

- `frontend/src/features/operations/Operations.module.css` — 新增 `.stageBlock`、`.stageTable` 两个类（放在 `.tChips` 之后、`.eventDetail` 之前，与阶段渲染就近）。`.tHead` 一字不动。
- `frontend/src/features/operations/index.tsx` — `:522` 阶段区块 `styles.tHead` → `styles.stageBlock`；`:524` 嵌套表 `styles.table` → `styles.stageTable`；`:370-378` 补 `<th>操作</th>`。
- `frontend/src/app/Shell.module.css` — `:195` `.main` 加 `min-width: 0`。
- `frontend/src/__tests__/features/operations/operations.test.tsx` — 新增 3 条结构断言测试。

---

## Task 1: 补齐 runs 表 `<th>操作</th>`（列数一致）

**Files:**
- Modify: `frontend/src/__tests__/features/operations/operations.test.tsx`
- Modify: `frontend/src/features/operations/index.tsx`

**可验证性:** 结构断言，可自动化。

- [ ] **Step 1: 先写失败测试（TDD 红）**

在 `operations.test.tsx` 的 `describe("运行日志 runs tab + tier 遥测(C6+C9)")` 内新增：

```tsx
it("表头列数与数据行列数一致（避免列错位）", () => {
  mountRunsTab([
    {
      id: "r1",
      runId: "80b22bb5-3eb3-4c18-b341-cd6122225f61",
      status: "outbox_enqueued",
      triggerKind: "inbound",
      createdAt: new Date().toISOString(),
    },
  ]);
  const table = document.querySelector("table")!;
  const th = table.querySelectorAll("thead th").length;
  const td = table.querySelectorAll("tbody tr:first-child > td").length;
  expect(th).toBe(td);
});
```

运行 `cd frontend && npx vitest run src/__tests__/features/operations/operations.test.tsx`，确认该条 **FAILED** 且报 `expected 5 to be 6`（不是别的原因）。

- [ ] **Step 2: 最小实现（TDD 绿）**

`index.tsx:370-378` 的 `<thead><tr>` 内，在 `<th>时间</th>` 之后补一列：

```tsx
<th>操作</th>
```

对齐既有惯例——跟进任务表（`:258-292`）就有 `<th>操作</th>`。

- [ ] **Step 3: 验证**

`npx vitest run src/__tests__/features/operations/operations.test.tsx` → 新测试 pass，既有 4 条 runs 测试仍 pass。

---

## Task 2: 阶段区块换用 `.stageBlock`（修主因）

**Files:**
- Modify: `frontend/src/__tests__/features/operations/operations.test.tsx`
- Modify: `frontend/src/features/operations/Operations.module.css`
- Modify: `frontend/src/features/operations/index.tsx`

**可验证性:** 类名断言可自动化；`display` 属性值不可（jsdom 不注入 CSS Module 样式）。

- [ ] **Step 1: 先写失败测试（TDD 红）**

同一个 describe 内新增：

```tsx
it("阶段区块不复用事件时间线的 tHead（后者是 flex 横向容器，会挤压标签并撑破表格）", async () => {
  mountRunsTab([
    {
      id: "r1",
      runId: "run-1",
      status: "outbox_enqueued",
      triggerKind: "inbound",
      createdAt: new Date().toISOString(),
      planner: { riskLevel: "medium", reviewMode: "full" },
    },
  ]);
  fireEvent.click(screen.getByText("展开"));
  const label = await screen.findByText("规划");
  const block = label.parentElement!;
  expect(block.className).not.toMatch(/tHead/);
  expect(block.className).toMatch(/stageBlock/);
  // 嵌套表必须用专用类（table-layout:fixed + word-break），不能用通用 .table
  const nested = block.querySelector("table")!;
  expect(nested.className).toMatch(/stageTable/);
});
```

确认 **FAILED** 且报 `expected '_tHead_xxxxx' not to match /tHead/`。

- [ ] **Step 2: 加 CSS 类（无测试，纯样式）**

`Operations.module.css` 在 `.tChips span { ... }`（`:88-91`）之后插入：

```css
/* 运行日志阶段明细：纵向堆叠（标签在上、表格在下）。
   刻意不复用 .tHead —— 后者是事件时间线的 flex 左右分列容器，
   把 width:100% 的表格塞进 flex 项会因 min-width:auto 撑破布局并挤压标签。 */
.stageBlock { margin-top: 12px; min-width: 0; }
.stageBlock > strong {
  display: block; margin-bottom: 6px;
  font-size: 13px; color: var(--ink-1); font-weight: 600; letter-spacing: -.2px;
}

/* 固定列宽 → 各阶段 key 列对齐；长值 word-break 折行而非撑列。 */
.stageTable { width: 100%; table-layout: fixed; border-collapse: collapse; font-size: 12.5px; }
.stageTable td {
  padding: 8px 12px; border-bottom: 1px solid var(--hairline);
  color: var(--ink-1); letter-spacing: -.1px; vertical-align: top;
  word-break: break-word; overflow-wrap: anywhere;
}
.stageTable td:first-child { width: 38%; color: var(--ink-3); }
.stageTable tbody tr:last-child td { border-bottom: none; }
```

- [ ] **Step 3: 换类名（TDD 绿）**

`index.tsx:522` 与 `:524`：

```tsx
<div key={key as string} className={styles.stageBlock}>
  <strong>{label}</strong>
  <table className={styles.stageTable}>
```

- [ ] **Step 4: 验证**

新测试 pass；既有 4 条 runs 测试仍 pass；`grep -n "tHead" index.tsx` 应只剩 `:308`（事件时间线那一处）。

---

## Task 3: `.main` 加 `min-width: 0`（全局守卫）

**Files:**
- Modify: `frontend/src/app/Shell.module.css`

**可验证性:** 不可自动化（CSS 属性值 + 跨页面布局）。靠全量测试不回归 + build 成功 + 部署后逐页目视。

- [ ] **Step 1: 改 CSS（无测试，纯样式）**

`Shell.module.css:195`：

```css
.main { flex: 1; min-width: 0; padding: 32px 44px; overflow-y: auto; height: 100vh; }
```

理由注释加在上一行：

```css
/* min-width:0 —— .shell 是 flex 容器，.main 作为 flex 项默认 min-width:auto，
   不肯收缩到内容宽度以下；任一页面出现宽内容（宽表格/长串）时其固有宽度会
   一路顶上去撑破整页布局。这一行把溢出约束在主区内。 */
```

- [ ] **Step 2: 验证不回归**

- `npx vitest run` 全量 → 618 tests / 125 files 全绿
- `npx tsc --noEmit` → 无错
- `npm run build` → 成功
- **风险说明**：这是唯一影响所有页面的改动。若全量测试出现任何布局相关断言失败，回退本任务并改为在 operations 页面局部加 `overflow-x:auto` 容器（代价：其他页面遇宽内容仍会撑破）。

---

## Task 4: 长值折行回归测试

**Files:**
- Modify: `frontend/src/__tests__/features/operations/operations.test.tsx`

**可验证性:** 结构断言（值被完整渲染、落在 `.stageTable` 内），可自动化。

- [ ] **Step 1: 写测试**

```tsx
it("长 JSON 值完整渲染在 stageTable 内（由 word-break 折行，不截断信息）", async () => {
  const ids = ["68a1f2c4d5e6a7b8c9d0e1f2", "68a1f2c4d5e6a7b8c9d0e1f3"];
  mountRunsTab([
    {
      id: "r1",
      runId: "run-1",
      status: "outbox_enqueued",
      triggerKind: "inbound",
      createdAt: new Date().toISOString(),
      knowledgeRoute: { selectedChunkIds: ids },
    },
  ]);
  fireEvent.click(screen.getByText("展开"));
  const cell = await screen.findByText(JSON.stringify(ids));
  expect(cell.closest("table")!.className).toMatch(/stageTable/);
});
```

- [ ] **Step 2: 验证**

应直接 pass（Task 2 已提供 `.stageTable`）。若 fail，说明 `renderStageValue` 的输出与断言不符——按实际输出修断言，不要改生产代码。

---

## Task 5: 全量校验与部署后目视复核

**Files:** 无（校验任务）

- [ ] **Step 1: 前端全量**

```bash
cd frontend
npx tsc --noEmit
npx vitest run
npm run build
```

预期：tsc 无错；621 tests（618 + 新增 3）全绿；build 成功。

- [ ] **Step 2: lint 红线**

```bash
bash scripts/check-no-human-takeover.sh origin/main HEAD
```

预期 `0 violations`。

- [ ] **Step 3: 部署后目视复核（不可自动化部分）**

按截图同一路径：运营 → 任务、事件与复核 → 运行日志 → 展开任一条。确认：

1. 阶段标签（规划/上下文/知识路由/决策/复核/送达网关）**横向**显示，不再逐字竖排
2. 无横向滚动条，内容不超出卡片右边界
3. 六个阶段的 key 列左右对齐
4. 长值（`selectedChunkIds` 等）折行完整可见
5. 事件时间线 tab（运营事件）的标签+时间仍左右分列——确认 `.tHead` 没被破坏

---

## Verification Summary

| 层 | 验证方式 | 可自动化 |
| --- | --- | --- |
| Task 1 列数一致 | `th` / `td` 计数断言 | ✅ |
| Task 2 类名换用 | className 断言 + `grep` | ✅ |
| Task 2 CSS 属性值 | 部署后目视 | ❌ |
| Task 3 全局守卫 | 全量测试不回归 + build | 部分 |
| Task 4 长值折行 | 值完整渲染 + 容器类名断言 | ✅ |
| 事件时间线未被破坏 | `grep` 确认 `.tHead` 仅剩 `:308` + 目视 | 部分 |

**Design doc:** `docs/superpowers/specs/2026-08-05-run-log-stage-table-overflow-design.md`

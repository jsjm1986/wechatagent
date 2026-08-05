# 运行日志阶段明细表格横向溢出修复

> 日期：2026-08-05
> 范围：前端「任务、事件与复核 → 运行日志」tab 的 run envelope 展开区
> 基线：`1cac218`（main）

## 问题

展开某条 run envelope 后，六个阶段区块（规划／上下文／知识路由／决策／复核／送达网关）的
key-value 表格横向溢出屏幕；阶段标签被压成逐字竖排（「知识路由」显示为「知识／路由」两行）。
溢出逃逸到页面级，侧边栏区域一并横移。

## 根因（四层，逐层已在源码中确认）

| 层 | 缺陷 | 位置 |
| --- | --- | --- |
| 1 | `.tHead` 类名误用：flex 容器内放 `width:100%` 的嵌套 `<table>` | `features/operations/index.tsx:522` |
| 2 | `.main` 为 flex 项但缺 `min-width: 0`，宽内容的固有宽度顶破整页 | `app/Shell.module.css:195` |
| 3 | `<thead>` 5 个 `<th>`，摘要行 6 个 `<td>`（缺「操作」列表头） | `features/operations/index.tsx:370-378` |
| 4 | `renderStageValue` 对对象走 `JSON.stringify`，产出无断点长单行 | `features/operations/index.tsx:466` |

第 1 层是主因。`.tHead`（`Operations.module.css:83`，`display:flex; justify-content:space-between`）
是为**事件时间线**设计的——`index.tsx:308` 用它承载 `<strong>标签</strong>` + `<span>时间</span>`，
两个小元素左右分列，符合设计意图。运行日志的阶段区块复用了同一个类，但第二个子元素是
`width:100%` 的表格；flex 项默认 `min-width:auto`，表格拒绝收缩到内容宽度以下，于是向外撑破，
同时把 `<strong>` 挤到最小宽度导致中文逐字换行。

第 2 层决定了溢出的影响半径。`.shell` 是 `display:flex`，`.main` 是 `flex: 1` 但没有
`min-width: 0`，因此子元素的固有宽度可以一路顶到页面级，而不是被约束在主内容区内。

## 已确定的取舍

**长值显示**：自动换行、完整可见。不截断、不省略号、不横向滚动条。
与既有惯例一致——`.eventDetail pre`（`Operations.module.css:98-102`）已采用
`white-space: pre-wrap; word-break: break-word`。代价是长数组行高较高，可接受。

**列宽**：固定列宽，六个阶段的 key 列对齐。用 `table-layout: fixed` + key 列 38%，
过长 key 折行而非撑列。相比按内容自适应，跨阶段视觉更稳定。

## 设计

### CSS（`features/operations/Operations.module.css`）

新增 `.stageBlock` / `.stageTable`，**不修改 `.tHead`**——事件时间线在正确使用它。

```css
.stageBlock { margin-top: 12px; }
.stageBlock > strong {
  display: block; margin-bottom: 6px;
  font-size: 13px; color: var(--ink-1); font-weight: 600; letter-spacing: -.2px;
}
.stageTable { width: 100%; table-layout: fixed; border-collapse: collapse; font-size: 12.5px; }
.stageTable td {
  padding: 9px 12px; border-bottom: 1px solid var(--hairline);
  vertical-align: top; word-break: break-word; overflow-wrap: anywhere;
}
.stageTable td:first-child { width: 38%; color: var(--ink-3); }
.stageTable tr:last-child td { border-bottom: none; }
```

`.stageBlock` 纵向堆叠（默认 block 流），标签独占一行，表格独占一行——彻底消除 flex 挤压。

### 全局守卫（`app/Shell.module.css`）

```css
.main { flex: 1; min-width: 0; padding: 32px 44px; overflow-y: auto; height: 100vh; }
```

唯一的全局改动。它让任何页面的宽内容被约束在主区内，而不是撑破整页布局。
这是本次唯一有连带风险的改动，需全量前端测试 + 逐页目视确认。

### 组件（`features/operations/index.tsx`）

1. 阶段区块的 `className` 从 `styles.tHead` 改为 `styles.stageBlock`；嵌套表格从
   `styles.table` 改为 `styles.stageTable`。
2. `<thead>` 补 `<th>操作</th>`，使 `th` 数（6）与摘要行 `td` 数（6）、展开行
   `colSpan={6}` 三者一致。对照组「跟进任务」表（`index.tsx:258-292`）已有 `<th>操作</th>`，
   本处属遗漏。

`renderStageValue` 的返回值格式不变——第 4 层由 `.stageTable td` 的 `word-break` 与
`table-layout: fixed` 共同兜住，无需改数据层。

## 不做的事

- 不重构 `renderStageValue`（例如改为可折叠 JSON 树）——超出本 bug 范围。
- 不修改 `.tHead` 与事件时间线渲染。
- 不改动其他 feature 的表格。

## 验证

**回归**：`运行日志 runs tab + tier 遥测(C6+C9)` describe 下现有 4 个测试必须继续通过
（含「展开后显式列出 tier 遥测三字段」）。

**新增测试**：
- `<thead>` 的 `th` 数与摘要行 `td` 数一致（锁住第 3 层，防再次漂移）。
- 阶段区块使用 `.stageBlock` 而非 `.tHead`（锁住第 1 层的类名选择）。
- 长 JSON 值场景下阶段表格带 `.stageTable`（锁住换行与固定列宽的载体）。

**全量**：`tsc --noEmit`、前端 618 个测试、production build。

**目视**：部署后按截图同一路径（运营页 → 运行日志 → 展开一条 inbound run）复核，
确认无横向溢出、阶段标签单行、长值折行。

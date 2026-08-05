# 统一收件箱（ask-human）频道布局对齐设计

**日期：** 2026-08-06
**范围：** `frontend/src/features/ask-human/`、`frontend/src/components/review/ReviewQueue.tsx`
**触发：** 用户反馈「卡片换行了，设计有点问题」，要求优化且保持项目风格一致性

## 1. 问题与根因

截图呈现四个现象，根因是同一个：**这个频道没有采用全项目统一的 `.page` + 白卡结构**，而是自建了一套外壳。

| 现象 | 根因 | 亲验证据 |
| --- | --- | --- |
| 9 个计数 chip 换行成两排 | 可用宽度不足（详见第 2 节测算） | `AskHuman.css:10` `max-width: 920px` |
| 整页松散、元素像浮在灰底上 | 全项目每个频道都用 `.page{display:grid;gap:18px}` + 内容包在白卡里；本频道**一张卡都没有** | 六个频道 `.module.css` 的 `.panel` 定义一致（见下）；`AskHuman.css` 无任何 panel 规则 |
| 内容区偏窄、边距偏大 | 双重 padding：Shell 的 `.main` 已有 `padding:32px 44px`，`.askHumanChannel` 又加 `24px 28px 40px`，并额外自设 `max-width:920px`（全项目唯一） | `Shell.module.css:199`、`AskHuman.css:5-11` |
| 两个按钮孤立贴在右上 | `.askHumanHeader{justify-content:flex-end}`，而 CSS 里的 `.askHumanHeader h1` 在 JSX 中**没有对应元素**（大页头归 Shell），左侧因此是空的 | `AskHuman.css:13-25` vs `index.tsx:180-197`（该段 `h1` 出现次数为 0） |
| 空态「暂无待处理项」是裸文字 | `reviewQueueEmpty` / `reviewQueueLoading` / `reviewQueueError` / `reviewQueueList` 四个 class **全库零 CSS 定义** | 全库仅 `ReviewQueue.tsx:93` 等处引用，无任何样式规则 |

已确认采用 `.page`+`.panel` 结构的六个频道：`Quality`、`Autonomy`、`Campaign`、`Operations`、`LlmProviders`、`Evolution`。其 `.panel` 定义高度一致：

```css
.page { display: grid; gap: 18px; }
.panel {
  border-radius: var(--r-lg);
  padding: 22px 26px 20px;
  background: var(--surface-card);
  border: 1px solid var(--hairline);
  box-shadow: 0 14px 34px -24px rgba(20, 30, 60, .3), inset 0 1px 1px rgba(255, 255, 255, .9);
}
```

## 2. 宽度测算（决定 chip 能否单排）

按 12px 字号估算：中文字符约 1em（12px），ASCII 约 0.5em（6px）。每个 chip 标签均为「4 个中文 + `: ` + 1 位数字」≈ 68px，加 `padding:6px 12px`（24px）与 `border` 2px ≈ **94px**。

```
9 chip × 94px          = 847px
8 个 gap × 8px         =  64px
合计需求               = 911px
```

可用宽度（`--sidebar-width: 282px`，`.main` 左右 padding 共 88px）：

| 视口 | 理论可用 | 当前实际（受 max-width:920 + 自身 padding 56 限制） |
| --- | --- | --- |
| 1440px | 1070px | **864px** ← 需 911px，差 47px，必然折行 |
| 1280px | 910px | 854px |

**结论：** 去掉 `max-width:920px` 与自身 padding 后，1440px 视口可用 1070px。将 chip 横向 padding 由 12px 收到 10px，单个 chip 降至约 90px，总需求降至 **875px**，1280px 视口（可用 910px）亦可单排，余量 35px。

这个测算同时否决了「chip 与两个按钮同处一排」的做法：911 + 按钮约 200 + 间距 ≈ 1111px > 1070px，放不下。故 chip 独占一行，按钮留在面板头部。

**测算基于字符宽度估算，非浏览器实测**，最终单排效果必须目视确认。

## 3. 设计

### 3.1 结构

```
.askHumanPage (display:grid; gap:18px)        ← 去掉 padding / max-width，交给 Shell
└─ .askHumanPanel (白卡)
   ├─ .askHumanPanelHead   左：待处理 N 项   右：已裁决历史 · 刷新
   ├─ .askHumanToolbar     9 个 chip 横向单排
   └─ 列表 / 空态
```

**命名必须带 `askHuman` 前缀。** `AskHuman.css` 是 plain CSS（全局作用域，非 CSS Module），而 `styles.css:423/444` 已定义全局 `.panel` 与 `.panelHead`，且**确有活跃消费者**（`features/user-ops/legacy.tsx:580/890/1099/1100/1152/1315`、`features/user-ops/cockpit/CockpitPanel.tsx:110`）。若直接命名 `.panel`，会与用户运营频道相互污染。

`AskHuman.css` 必须保持 plain `.css`——文件头注释已警告：改成 `.module.css` 做副作用导入会被 Rollup tree-shake 删光，整频道白板。

### 3.2 面板头部左侧放「待处理 N 项」

Shell 已提供大页头（eyebrow `ASK-HUMAN` + 标题「统一收件箱」+ 副标题），面板内再写一遍标题是重复。改放 `summary.total`——这是本页最该被一眼看到的数字，同时让两个按钮不再孤立飘在右上。

`total` 的类型是 `number | null`（`lib/inboxApi.ts:38`，解析见 `:105`：非 number 一律取 null）。为 null 时不显示计数，只渲染按钮组，不显示「待处理 0 项」（那是错误信息——null 表示计数不可用，不是没有待办）。数据已在 store 中，无需新增接口。

### 3.3 chip 横向单排

`.askHumanToolbar` 采用 `display:flex; flex-wrap:wrap; gap:8px`，与 `Quality`/`Autonomy` 的 `.toolbar`（`display:flex; align-items:center; flex-wrap:wrap; gap:10px`）同形。chip 横向 padding 由 12px 收到 10px。

`flex-wrap` **保留**，但语义从「常态折行」变为「更窄屏的溢出兜底」——这是防溢出保险，不是预期布局。

### 3.4 空态改用共享 EmptyState

现状是裸 `<div className="reviewQueueEmpty">`，而该 class 无任何 CSS。项目已有共享组件 `components/ui/EmptyState`（`LlmProviders`、`SendAnalytics`、`ReferralCards` 均在用）：虚线框 + 图标 + 标题 + 提示，`background: var(--surface-page)`。

`ReviewQueue` 的 `emptyText?: string` 参数改为可接收 ReactNode，由 ask-human 侧传入 `<EmptyState>`；保持默认值 `"暂无待处理项"` 以兼容其它潜在调用方。同时给 `reviewQueueLoading` / `reviewQueueError` 补最小样式。

**`reviewQueueList` 这个 class 名必须保留**——测试 `AskHumanView.dataSource.test.tsx:79` 用 `container.querySelector(".reviewQueueList")` 定位列表。

**范围安全性：** `ReviewQueue` 组件全库只有 ask-human 一个真实消费者（`features/knowledge/steward.tsx` 中的 `ReviewQueueItem` / `ReviewQueueResponse` 只是同名 TypeScript 类型，并未 import 该组件）。故为其补样式不外溢。

### 3.5 顺手修正

- `var(--surface-1, #fff)` 共 3 处（`AskHuman.css:80/147/194`）——`--surface-1` 在全库 CSS 中**无定义**，实际一路 fallback 到硬编码 `#fff`，与该文件头注释「禁止硬编码」自相矛盾。改为 `var(--surface-card)`。
  （`:302` 的 `var(--surface, #fff)` 不在此列：`--surface` 确有定义于 `styles.css:4`，无需改动。）
- 删除 `.askHumanHeader h1` 死规则（JSX 无对应元素）。

## 4. 测试

jsdom 无布局引擎、不跑 CSS 层叠，`flex-wrap` 实际是否折行、宽度、颜色**均不可断言**。测试只锁结构与文本：

1. 内容渲染在白卡容器内（`.askHumanPanel` 存在）
2. 9 个 chip 全部位于同一个 `.askHumanToolbar` 容器内（数量为 9）
3. `summary.total` 有值时显示「待处理 N 项」
4. `summary.total` 为 null 时不显示计数（不出现「待处理 0 项」）
5. 空态渲染 `EmptyState` 结构（虚线框容器 + 文案），而非裸 div

真实单排效果、白卡观感需目视确认（清单见第 6 节）。

## 5. 不做的事

- 不动 9 个来源的语义与 `SOURCE_META` / `SOURCE_TONE` 映射
- 不动 chip 点击切源逻辑与 `refreshNonce` / `setActiveSource` 时序——`index.tsx:167-173` 有精心写就的死循环规避注释（`fetchItems` 必须 memoize），碰它风险远大于收益
- 不重构 `ReviewQueue` 组件内部的 generation / busyId 并发控制
- 不动 `ResolvedEscalations`（已裁决历史）视图的内部样式
- 不新增竖排筛选栏：左侧已有频道栏，再立一条平行竖列会混淆层级，且违反 `docs/frontend-design-system.md`「Do not add a third persistent navigation level」

## 6. 需目视核验

- 1440px 与 1280px 视口下 9 个 chip 均为横向单排，不折行
- 内容包在白卡内，不再直接贴在灰底上；卡片观感与「运营成效中心」「自治回路」等频道一致
- 面板头部左侧「待处理 N 项」与右侧两个按钮在同一行、垂直居中
- 空态呈现为虚线框 + 图标 + 文案，不再是裸文字
- 窗口收窄至 1100px 以下时 chip 折行仍整齐（兜底路径）
- 已裁决历史视图切换后布局未被破坏

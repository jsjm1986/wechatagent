# 系统策略「标签与状态」tab 三面板分页设计

## 背景与问题（playwright 线上实测）

2026-07-08 用 playwright 登录 117 线上（admin/admin）实测「系统策略 → 标签与状态」tab 的真实高度：

| 面板 | 组件 | 实测高度 | 条目数 |
| --- | --- | --- | --- |
| 状态机策略灰度 | `StatePolicyAdmin`（index.tsx:555） | 225px（空态） | 0 |
| 双层标签字典 | `TaxonomiesAdmin`（:641） | 4942px | 19 |
| **新词候选审核** | **`TaxonomyCandidatesAdmin`（:1003）** | **104725px（116 屏）** | **176 pending** |

整页滚动容器 `_main` scrollHeight = **110181px ≈ 122 屏**，其中候选审核面板一家占 95%。

**根因**：三个面板都把 `items` **一次性全量 `map` 平铺**（`<div className={styles.versionedList}>{items.map(...)}</div>`），无分页无限高。候选面板 176 条 pending 是主凶；字典 19 条已偏长；状态机当前空但同样无上限（未来堆积同样炸）。

#154「历史版本默认收起」只作用于字典/状态机的**版本维度**，对候选面板（非版本列表）完全无效，也不解决 176 条**条目维度**的平铺。

## 目标

给「标签与状态」tab 的三个 Admin 面板各加**客户端分页**（每页 20 条），彻底消除长度随条目数无限增长。复用仓内既有分页模式（`CampaignBoard.tsx:55-146` + `Campaign.module.css:81-89` 的 `.pager/.pagerBtn/.pagerInfo`），不新造视觉、不改后端 API、不改各面板的 item 渲染内容。

## 复用的既有模式（CampaignBoard，仓内权威范式）

```tsx
const PAGE_SIZE = 20;
const [page, setPage] = useState(0);
const pageCount = Math.max(1, Math.ceil(items.length / PAGE_SIZE));
const safePage = Math.min(page, pageCount - 1);      // 越界自愈：filter/reload 后条目变少不会停在空页
const pageRows = items.slice(safePage * PAGE_SIZE, safePage * PAGE_SIZE + PAGE_SIZE);
// 列表 map 改用 pageRows；列表尾部：
{pageCount > 1 && (
  <div className={styles.pager}>
    <button type="button" className={styles.pagerBtn} disabled={safePage === 0} onClick={() => setPage(safePage - 1)}>上一页</button>
    <span className={styles.pagerInfo}>{safePage + 1} / {pageCount}</span>
    <button type="button" className={styles.pagerBtn} disabled={safePage >= pageCount - 1} onClick={() => setPage(safePage + 1)}>下一页</button>
  </div>
)}
```

**为何用 `safePage` 而非在 reload/filter 时 `setPage(0)`**：`safePage = Math.min(page, pageCount-1)` 在渲染期夹取，条目数变化（切候选 status filter、勾显示历史版本、reload 后变少）时页码自动落到最后有效页，无需在每个 setState 处手动重置，也不会因 stale page 停在空白页。这与 CampaignBoard 一致。

## 三面板改动点（逐一）

三个面板结构同构：`items` state + `reload()` + `!loading && items.length===0 && <Empty/>` + `<div className={styles.versionedList}>{items.map(...)}</div>`。改动一致：

1. **`StatePolicyAdmin`（:555）**：加 page state + PAGE_SIZE；`items.map` → `pageRows.map`；`versionedList` 后加 pager。
2. **`TaxonomiesAdmin`（:641）**：同上。注意它 map 内有编辑态（editingId）——分页只切渲染的行，编辑态 state 不变（编辑中的行若翻页会移出视图，属可接受行为，不特殊处理，YAGNI）。
3. **`TaxonomyCandidatesAdmin`（:1003）**：同上，主凶。它有 statusFilter，切 filter 后 items 变化由 safePage 自愈。

**空态与分页的关系**：`items.length===0 → <Empty/>`（pageCount=1，pager 因 `pageCount>1` 为假不显示）。`0 < items.length ≤ 20 → 一页，无 pager`。`>20 → 显示 pager`。

## CSS（`SystemStrategy.module.css`）

新增 `.pager / .pagerBtn / .pagerInfo`，**复刻** `Campaign.module.css:81-89` 的定义（同 token：`--r-sm`、`--ink-2/3`、`--surface-card`、`--hairline`、蓝色 hover `rgba(10,132,255,.3)`）。不跨模块 import（CSS Module 边界红线），复刻而非借用。

## 不做（YAGNI）

- 不改后端 API（继续一次性拉全量，仅前端切片）。176 条全量 JSON 不大，不值得引入后端分页 + 游标契约。
- 不做「每页条数可调」下拉、不做跳页输入框、不做首页/末页按钮——CampaignBoard 就只有上一页/页码/下一页，保持一致。
- 不做二级 tab / accordion（用户已否决，选定分页）。
- 不改 item 卡片内容、不改 reload/filter/编辑/废弃逻辑。
- 不给 tab 分区结构（#153 已做）、不动 control/profile/lessons 三个 tab。

## 测试同步

现有 `systemStrategy.test.tsx` / `taxonomyFlags.test.tsx` 用少量 seed（1-2 条）断言，全部落在第一页，**分页后行为不变**（≤20 条无 pager），不需改断言。新增：
- 一个「候选 >20 条时只渲染 20 条 + 显示 pager + 点下一页翻页」的用例（对 `TaxonomyCandidatesAdmin`，mock api.get 返回 25 条，断言渲染条数 ≤20、pager 出现、点「下一页」后第 21 条可见）。
- 一个「≤20 条不显示 pager」的用例（边界：正好 20 条无 pager，21 条有）。

## 验证

1. `npx tsc --noEmit` → 0 error。
2. `npx vitest run` → 全绿（既有 + 新增分页用例）。
3. `bash scripts/check-no-human-takeover.sh` → 0 violations（新增文案「上一页/下一页」无禁用词）。
4. playwright 复测 117（合并部署后）：「标签与状态」tab 整页高度从 ~110000px 降到单页 < 6000px（三面板各 ≤20 条 + pager）。

## 落地流程

纯前端：单组件文件（index.tsx 三处函数）+ 单 CSS + 一个测试文件新增用例。走 TDD（先写分页用例=红 → 三面板加分页=绿）→ 三门 → PR → CI 前端契约门 → 合并 → 部署 117 重建 dist → playwright 复测高度。

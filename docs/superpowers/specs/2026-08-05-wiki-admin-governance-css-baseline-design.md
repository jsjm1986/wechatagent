# 知识库 Wiki · 治理工坊样式基线修复设计

- 日期：2026-08-05
- 范围：`frontend/src/styles.css`、`frontend/src/features/knowledge/Knowledge.css`、`frontend/src/features/knowledge/atlas.tsx`
- 缺陷来源：管理员在「知识库 Wiki → 控制台 → 治理工坊 → 分类系统」截图中发现五处渲染问题

## 一、问题陈述

治理工坊面板存在五处渲染缺陷，其中一处是功能缺陷（高危按钮不可见），其余四处是可读性缺陷。

缺陷范围比截图更大。`atlas.tsx` 的四个治理面板共享同一套 `.wikiAdmin*` 样式：

| 面板 | 定义位置 | 表格列数 | 裸「刷新」按钮 | 裸 ISO `updatedAt` | PublishBar |
| --- | --- | --- | --- | --- | --- |
| `MetadataDashboard` | `atlas.tsx:810` | 无表格 | 有 | 无 | 无 |
| `TaxonomiesGovernance` | `atlas.tsx:1127` | 9 | 有 | 有 | 有 |
| `StatePoliciesGovernance` | `atlas.tsx:1230` | 6 | 有 | 有 | 有 |
| `DomainGovernance` | `atlas.tsx:1311` | 5 | 有 | 有 | 有 |

因此白底白字、按钮尺度失调、表格挤压、ISO 时间四类问题在多个面板同时存在，截图只呈现了其中一个。

## 二、根因

`Knowledge.css` 中 `.wikiAdmin*` 这批规则（`:3096`–`:3200`）是按「从零编写样式」的方式写的，但它实际运行在 `styles.css` 全局元素基线之上，只覆盖了部分属性——未覆盖的属性就露出全局值：

- 漏 `color` → 白底白字（`Knowledge.css:3177`）
- 漏 `width` → checkbox 撑满行宽（`styles.css:563`）
- 整个未覆盖 → 38px 纯蓝按钮（`styles.css:71`）

同一文件 `:1555` 的 `.wikiDigestActions button` 是做对的样板：显式声明了 `background` 与 `color`。

## 三、逐项设计

### 1. 全局基线：`input{width:100%}` 排除 checkbox / radio

`styles.css:563` 的 `input, textarea { width: 100% }` 对 checkbox 同样生效。紧邻的 `:572` 已用 `input:not([type="checkbox"]):not([type="radio"])` 把 `height` 排除，`width` 却没有。

改法：给 width 规则补上同样的 `:not()` 选择器，两条规则口径一致。

这是本次唯一的全局改动，安全性已逐项核验：

- 全库 38 处 `type="checkbox"`，其中 6 处已显式写 `width: 16px / 15px / auto` 与全局规则对抗（`CommandCenter.module.css:103` 直接写 `width: auto`；另有 `SystemStrategy.module.css:146`、`EvolutionCenterTab.module.css:72`、`ContentAssets.module.css:113`、`LlmProviders.module.css:187`、`styles.css:2018`）
- **0 处依赖 `width: 100%`**
- radio 全库 0 处使用
- `appearance: none` 的元素全是 `<select>`，不受影响
- `RosterView.module.css:63` 的 `.checkbox` 作用于 `<div>`（`RosterView.tsx:241`），不是 input

符合 `docs/frontend-design-system.md:162`「Inputs are full width」的原意——该条指文本输入框，不含勾选框。这 6 处既有覆盖是同一个坑被反复局部规避的证据；修基线后它们成为冗余但无害，本次不动，避免扩大改动面。

### 2. PublishBar 白底白字（功能缺陷）

`Knowledge.css:3177` 的 `.knowledgeWiki .wikiPublishBar button` 覆盖了 `background: var(--surface-card)`（白）但未声明 `color`，于是继承 `styles.css:71` 的 `color: #fff` → 白底白字。

同排另两个按钮可见，是因为各自 class 覆盖了 color：`发布新版` 有 `wikiActionBtn--verify`（`:2619`，蓝），`回退上版` 有 `wikiActionBtn--reject`（`:2627`，红）。中间的「发布给全部」（`atlas.tsx:1051`）没有 class，因此漏掉。

这一处优先级最高：它是不可逆高危操作。`atlas.tsx:1010` 的确认文案为「将把新版本推送给全部会话，立即对所有客户生效，且不可逆」。该按钮当前渲染为一个不可见的空白框。它有 `requireText: "确认发布"` 二次输入兜底，不至于误触即生效，但按钮本身不应隐形。

**不能**在 `.wikiPublishBar button` 规则内直接补 `color`。特异性算过了：

| 选择器 | 特异性 | 说明 |
| --- | --- | --- |
| `.knowledgeWiki .wikiPublishBar button` | (0,2,1) | 2 类 + 1 元素 |
| `.knowledgeWiki .wikiActionBtn--verify` | (0,2,0) | 2 类 |

类计数打平在 2，元素选择器让 `.wikiPublishBar button` **胜出**。在该规则里加 `color` 会把蓝色的「发布新版」和红色的「回退上版」一起刷成灰色——修一个缺陷换来两个回归。

改法：给「发布给全部」按钮补一个语义 class `wikiActionBtn--neutral`（全库未占用，已核验），规则与 `--verify` / `--reject` 同级：

```css
  .knowledgeWiki .wikiActionBtn--neutral {
    color: var(--ink-2);
  }
```

`border` 无需声明——`.wikiPublishBar button` 已给出 `1px solid var(--surface-page)`。

不用 `.wikiPublishBar button:not([class])`：它虽然能只命中这一个按钮，但语义是「没有任何 class 的按钮」，将来任何人给该按钮加一个无关 class（如埋点标记）就会让颜色再次消失，是个潜伏陷阱。走 class 方案后，PublishBar 三个按钮各自有显式 color，形态一致。

### 3. 工具栏按钮尺度

`.wikiAdminToolbar`（`Knowledge.css:3132`）内的「刷新」按钮无 class，完整继承 `styles.css:71`：`min-height: 38px`、`background: #175cd3`、`padding: 8px 13px`、`font-size: 13px`、`font-weight: 680`。同屏 PublishBar 按钮是 11px / `padding: 4px 8px` 的白底描边款，尺度冲突明显。

改法：新增 `.knowledgeWiki .wikiAdminToolbar button` 规则，对齐同面板 `.wikiPublishBar button` 的视觉规格，并显式声明 `color` 与 `min-height: auto` 压掉全局值。

但单条 `.wikiAdminToolbar button` 只覆盖三个面板——四个「刷新」按钮的容器并不相同：

| 面板 | 按钮位置 | 容器 class |
| --- | --- | --- |
| `TaxonomiesGovernance` | `atlas.tsx:1167` | `.wikiAdminToolbar` |
| `StatePoliciesGovernance` | `atlas.tsx:1257` | `.wikiAdminToolbar` |
| `DomainGovernance` | `atlas.tsx:1338` | `.wikiAdminToolbar` |
| `MetadataDashboard` | `atlas.tsx:865` | `.wikiArchiveHeaderActions` |

第四个不能用 `.wikiArchiveHeaderActions button` 兜——该 class 另在 `today.tsx:376`、`today.tsx:581`、`steward.tsx:1732`、`steward.tsx:2419` 使用，规则会溢出到本次范围外的面板（其中 `steward.tsx:1758` 那个按钮已有 `.ghost wikiBtn` 款式）。改用 `.wikiMetadataDashboard`（`atlas.tsx:859`，全库唯一使用处）精确限定。

不复用全局 `button.secondary`（`styles.css:108`）：它只改 `border-color` / `background` / `color`，不改 `min-height`，38px 仍会压着 11px 的表格行。

### 4. 表格列宽约束

`.wikiAdminTable`（`Knowledge.css:3146`）只有 `width: 100%`，无 `table-layout`，浏览器按内容自动分配列宽。9 列表格因此把表头「版本」竖排成两行、「当前生效」折成两截，行高被撑到约 93px；`.wikiPublishBar` 的 `flex-wrap: wrap`（`:3171`）让三个按钮折成两行。

改法：
- `.wikiAdminTable` 加 `table-layout: fixed`
- `.wikiAdminTable tbody td` 加 `word-break: break-word` 与 `overflow-wrap: anywhere`
- 三张表各加 `<colgroup>` 声明显式百分比列宽

用 `<colgroup>` 而非 CSS `nth-child`：三张表列数不同（9 / 6 / 5），`table-layout: fixed` 在无显式宽度时均分列宽，9 列均分会让 `✓` 这类窄列与「更新时间」同宽。列宽写在 JSX 里也让「加列时必须同步改 colgroup」这件事在同一处可见。

这与已合并的 #238（运行日志阶段表溢出）同源——同样是 `table-layout: fixed` + 显式列宽 + `word-break`。那次的修复落在 `Operations.module.css`，本表在另一套 class 中，未受益。

### 5. ISO 时间格式化

三处 `updatedAt` 直接渲染后端返回的 ISO 字符串，得到 `2026-06-26T07:25:11.049Z`，在列宽不足时于连字符处断成两行。三处均为 `<td className="wikiArchiveTimelineTime">{it.updatedAt ?? ""}</td>`：

| 位置 | 所属面板 |
| --- | --- |
| `atlas.tsx:1205` | `TaxonomiesGovernance` |
| `atlas.tsx:1287` | `StatePoliciesGovernance` |
| `atlas.tsx:1366` | `DomainGovernance` |

`MetadataDashboard`（`atlas.tsx:810`）不渲染 `updatedAt`，不在本项改动范围内。

改法：改为 `x ? new Date(x).toLocaleString() : "—"`，与同频道既有惯例一致（`steward.tsx:495`、`steward.tsx:2250`、`CampaignList.tsx:78`）。

不新增工具函数：`lib/format.ts` 目前只有 `formatRate` / `formatNumber` 两个数值函数，而日期渲染已有成型惯例，新造抽象会多出一个需要各处迁移的中间层。

## 四、测试策略

`frontend/src/__tests__/` 已有 vitest + @testing-library/react。

可自动化验证：
- 三个面板的 `updatedAt` 渲染为本地化时间，且 `undefined` 回退为 `"—"`
- 三张表的 `<colgroup>` 列数与 `<th>` 列数一致（9 / 6 / 5），防止将来加列漏改
- 「发布给全部」按钮带有 `wikiActionBtn--neutral` class（走 class 方案的附带收益：这一处从"只能目视"变成可自动化断言）

**无法自动化验证的部分**：jsdom 没有布局引擎，`table-layout`、`width`、`color` 的实际视觉结果取不到。`Knowledge.css` 是 plain CSS（非 CSS module，见该文件头注释：改为 `.module.css` 会被 Rollup tree-shake 掉整份样式导致频道裸奔），类名是字面量，因此结构与类名断言有效，但渲染效果必须目视确认。这与 #238 的情况相同，不在文档里假装覆盖。

第 1 条全局基线改动同样不可自动化验证，依据是上文枚举的 38 处核验结果，加上部署后抽查若干含 checkbox 的页面。

需目视确认的清单：
- 「发布给全部」按钮文字可见
- 「显示历史版本」label 单行显示，checkbox 为方形小框
- 「刷新」按钮与 PublishBar 按钮尺度协调
- 三张表列宽对齐、表头不竖排、行高恢复紧凑
- 更新时间为本地化格式且单行
- 抽查 `system-strategy`、`content-assets`、`command-center`、`evolution` 频道的 checkbox 未因基线改动变形

## 五、实施约束

CI 有 `scripts/check-no-human-takeover.sh` 门禁，扫描 `frontend/src/` 等路径新增行中的禁用词（含「人工」）。本次新增的测试与注释若要表达「需目视确认」，措辞需避开该词，用「目视确认 / 视觉核验」等表述。设计文档位于 `docs/`，不在扫描范围内。

## 六、不做的事

- 不重写 `.wikiAdmin*` 为 CSS module（会触发 tree-shake 导致频道样式全丢）
- 不清理第 1 条列出的 6 处冗余局部覆盖（与本次目标无关，扩大回归面）
- 不重构 `atlas.tsx` 四个治理面板的重复结构（同构重复是既有事实，但抽公共组件属独立改动）

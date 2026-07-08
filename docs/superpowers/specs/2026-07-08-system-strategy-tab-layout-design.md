# 系统策略频道 tab 分区改造设计

## 背景与问题

「系统策略」频道（`frontend/src/features/system-strategy/index.tsx`，`SystemStrategyInner`）把 7 个大 Admin 面板**全部平铺**在 `<div class={styles.page}>`（CSS `display:grid; gap:18px`，无 max-height / overflow / tab）里同时渲染：

1. 系统总控策略（inline section，`:2529`）
2. `DomainPromptPanel`（Soul + Prompt 模板编辑器，含 draft 表单，体量最大）
3. `StatePolicyAdmin`（状态机策略）
4. `TaxonomiesAdmin`（字典管理，含版本历史长列表）
5. `TaxonomyCandidatesAdmin`（新词候选审核）
6. `LessonsLearnedAdmin`（经验教训列表）
7. `DomainProfilePanel`（行业配置向导，含 profile 列表 + 编辑器）

每个面板又各含长列表，全部同时展开 → 页面无限长。这是**信息组织的架构缺陷**，非单点 CSS bug。

**对照证据**：其它复杂频道都有分区、一次只渲一个面板——`knowledge`（`useState<KnowledgeMode>` + `wikiModeBar` NavBtn + `{mode === x && <Panel/>}`）、`user-ops/cockpit`、`send-analytics`、`quality`、`products-deals`。唯独 system-strategy 平铺。

## 目标

按现有设计系统的 tab 模式，把 7 面板归为 **4 个职能 tab**，一次只渲染当前 tab 的面板，消除无限长；视觉与交互与全站一致。**不改任何面板组件的内部逻辑**——只重组 `SystemStrategyInner` 的顶层 + 加 tab bar 样式。

## 分组（用户确认：4 组）

| tab key | 中文标签 | 眉标(英文,保留) | 含面板 |
| --- | --- | --- | --- |
| `control` | 总控与 Prompt | Global Strategy | 系统总控 section + `DomainPromptPanel` |
| `taxonomy` | 标签与状态 | Taxonomy & State | `StatePolicyAdmin` + `TaxonomiesAdmin` + `TaxonomyCandidatesAdmin` |
| `profile` | 行业配置 | Domain Profile | `DomainProfilePanel` |
| `lessons` | 经验教训 | Lessons Learned | `LessonsLearnedAdmin` |

默认 tab = `control`。

## 设计

### 1. 顶层结构改造（`SystemStrategyInner`，`index.tsx`）

`SystemStrategyInner` 顶部加 tab 状态：
```tsx
type StrategyTab = "control" | "taxonomy" | "profile" | "lessons";
const [tab, setTab] = useState<StrategyTab>("control");
```

`return` 里 `<div className={styles.page}>` 内部改为：**tab bar + 条件渲染**。原本平铺的 7 个面板按分组表拆进 `{tab === "..." && (...)}` 块。所有面板的 props（`busy` + 各 handler）原样传入，**零内部改动**。

tab bar 用本频道 CSS Module 新增类（不借 knowledge 的全局 `wikiModeBar`——那是 `Knowledge.css` 私有全局类，跨频道复用违反 CSS Module 边界与「遵守现有设计系统」红线）：

```tsx
<div className={styles.tabBar}>
  {STRATEGY_TABS.map((t) => (
    <button
      key={t.key}
      type="button"
      className={`${styles.tabBtn} ${tab === t.key ? styles.tabBtnActive : ""}`}
      onClick={() => setTab(t.key)}
    >
      <t.Icon size={16} />
      <span>{t.label}</span>
    </button>
  ))}
</div>
```

`STRATEGY_TABS` 常量数组（key/label/Icon）。图标复用已 import 的 lucide 图标族（同频道现有风格：Settings2 等）。

**关键：`useEffect(loadStrategyData)` 保持在 `SystemStrategyInner` 顶层不动**——数据一次性加载，切 tab 不重新拉数据（各面板自己的 `useEffect` 拉各自数据的行为不变，因为组件挂载时机改为“切到该 tab 才挂载”，符合按需加载，无副作用回归）。

### 2. CSS（`SystemStrategy.module.css`）

新增 `.tabBar` / `.tabBtn` / `.tabBtnActive`，对齐设计 token（颜色/圆角/间距用现有 `--r-sm` 等变量，选中态用频道既有的主色纪律——蓝仅主操作，这里 tab 选中用中性强调不用蓝）。参照 `.profileTab`/`.profileTabActive`（本文件 `:1122` 已有的子 tab 样式）保持同款观感，避免新造视觉语言。

`.page` 保持 `display:grid; gap:18px` 不变（现在每次只有 1 个 tab 的内容 + tab bar，自然不再无限长）。

### 3. 测试同步（5 个文件，必须改）

现有测试直接 `render(<SystemStrategyFeature />)` 并断言面板内容，且 `systemStrategy.test.tsx:126` 注释明写「一次渲染全部面板，无需切 tab」——**tab 化后非默认 tab 的面板不再渲染，这些断言会全红**。每个测试需在 render 后、断言前**先 `fireEvent.click` 切到目标面板所在 tab**。

受影响文件与对应 tab：
- `systemStrategy.test.tsx`：TaxonomiesAdmin 用例 → 切「标签与状态」；DomainProfilePanel 用例 → 切「行业配置」
- `taxonomyFlags.test.tsx`：TaxonomiesAdmin → 「标签与状态」
- `domainProfileVersions.test.tsx`：DomainProfilePanel → 「行业配置」
- `promptConfirm.test.tsx`：DomainPromptPanel（prompt 保存/发布）→ 默认「总控与 Prompt」，可能无需切（确认默认 tab 含它即可）
- （候选卡自身单测 `TaxonomyCandidateReviewCard.test.tsx` 直接渲染卡片组件，不经 Feature，**不受影响**）

抽一个测试辅助 `selectTab(name)`（`fireEvent.click(screen.getByRole("button",{name}))`）减少重复。断言维度不变（只加“先切 tab”前置），非过拟合。

## 不做（YAGNI）

- 不改任何 Admin 面板组件的内部实现 / props / store。
- 不做 URL 路由持久化 tab（本 app 无 router lib，全站 tab 都是组件内 useState，保持一致）。
- 不做 accordion / 虚拟滚动 / 懒加载分页（tab 已解决无限长；长列表分页是各面板各自的独立议题，不在本次范围）。
- 不动眉标英文（沿用全站既定保留决定）。
- 不合并/拆分任何面板组件。

## 验证

1. `npx tsc --noEmit` → 0 error。
2. `npx vitest run`（全量）→ 全绿；重点确认 5 个 system-strategy 测试文件切 tab 后断言通过、无“多元素/找不到”回归。
3. 起 dev server 人工核对：4 个 tab 可切换，每个 tab 只显示该组面板，页面不再无限长；默认落「总控与 Prompt」。
4. `bash scripts/check-no-human-takeover.sh` → 0 violations（tab 标签「总控/标签与状态/行业配置/经验教训」无禁用词）。
5. 合并后部署 117 重建前端 dist，截图复验页面高度恢复正常。

## 落地流程

纯前端改动，单文件组件 + 单 CSS + 5 测试文件。走：写计划 → TDD（先改测试加 selectTab 前置=会红→改组件 tab 化=转绿）→ 三门 → PR → CI（前端契约门）→ 合并 → 部署 117 重建前端。

# 系统策略频道 tab 分区改造 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把「系统策略」频道的 7 个平铺 Admin 面板重组为 4 个职能 tab，一次只渲染当前 tab，消除页面无限长。

**Architecture:** 只改 `SystemStrategyInner` 顶层：加 `tab` useState + tab bar + 条件渲染，7 面板按 4 组归类。面板组件内部零改动。复用本频道已有的 `.profileTabBar`/`.profileTab`/`.profileTabActive` CSS 类，不新建视觉。同步 5 个直接 `render(<SystemStrategyFeature/>)` 的测试文件（加“先切 tab”前置 / 拆跨 tab 断言）。

**Tech Stack:** React 19 + TypeScript + Vite + Vitest + CSS Modules。

## Global Constraints

- 眉标英文保留（`Global Strategy` 等，用户 2026-07-04 既定全站装饰性英文眉标不译）。
- 前端遵守现有设计系统（`docs/frontend-design-system.md`）：复用 `SystemStrategy.module.css` 现有 tab 类，蓝色仅主操作、tab 选中态用频道既有 `.profileTabActive` 观感。
- 不改任何 Admin 面板组件的内部逻辑 / props / store；不做 URL 路由持久化；不做 accordion / 分页 / 懒加载。
- 三门必过：`npx tsc --noEmit` 0 error；`npx vitest run` 全绿；`bash scripts/check-no-human-takeover.sh` 0 violations。
- 测试只增量/同步锁串，断言维度不变（反过拟合）。

## 4 组映射（用户确认）

| tab key | 中文标签 | 含面板（源码顺序） |
| --- | --- | --- |
| `control` | 总控与 Prompt | 系统总控 section（`:2529`）+ `DomainPromptPanel`（`:2565`） |
| `taxonomy` | 标签与状态 | `StatePolicyAdmin`（`:2590`）+ `TaxonomiesAdmin`（`:2591`）+ `TaxonomyCandidatesAdmin`（`:2592`） |
| `profile` | 行业配置 | `DomainProfilePanel`（`:2594`） |
| `lessons` | 经验教训 | `LessonsLearnedAdmin`（`:2593`） |

默认 tab = `control`。

## 文件结构

- Modify: `frontend/src/features/system-strategy/index.tsx` — `SystemStrategyInner` 顶层加 tab；补 lucide 图标 import。
- Modify: `frontend/src/features/system-strategy/SystemStrategy.module.css` — 仅在必要时微调（优先纯复用 `.profileTabBar` 系列；若频道级 tab 需要更大字号/间距，加 `.channelTabBar` 变体）。
- Modify（测试同步）:
  - `frontend/src/__tests__/features/system-strategy/systemStrategy.test.tsx`
  - `frontend/src/__tests__/features/system-strategy/taxonomyFlags.test.tsx`
  - `frontend/src/__tests__/features/system-strategy/domainProfileVersions.test.tsx`
  - `frontend/src/__tests__/features/system-strategy/promptConfirm.test.tsx`

---

### Task 1: 测试同步——加 selectTab 前置 + 拆跨 tab 断言（先红）

**Files:**
- Modify/Test: `frontend/src/__tests__/features/system-strategy/systemStrategy.test.tsx`
- Modify/Test: `frontend/src/__tests__/features/system-strategy/taxonomyFlags.test.tsx`
- Modify/Test: `frontend/src/__tests__/features/system-strategy/domainProfileVersions.test.tsx`
- Modify/Test: `frontend/src/__tests__/features/system-strategy/promptConfirm.test.tsx`

**Interfaces:**
- Produces（供 Task 2 组件实现对齐）：tab 按钮以中文标签作为可点击文本——`总控与 Prompt` / `标签与状态` / `行业配置` / `经验教训`。测试用 `fireEvent.click(screen.getByRole("button", { name: "标签与状态" }))` 切 tab。组件必须让这些标签作为 `<button>` 文本可被 getByRole 命中。

- [ ] **Step 1: 在 systemStrategy.test.tsx 顶部加 selectTab 辅助**

在 import 区之后、第一个 describe 之前加：
```tsx
// tab 化后：非默认 tab 的面板要先切 tab 才渲染。默认 tab = 总控与 Prompt。
function selectTab(name: "总控与 Prompt" | "标签与状态" | "行业配置" | "经验教训") {
  fireEvent.click(screen.getByRole("button", { name }));
}
```
确认文件已 import `fireEvent`（`systemStrategy.test.tsx` 顶部已用 fireEvent，无需新增）。

- [ ] **Step 2: 拆 `:88` 跨 tab 断言用例**

原用例（一次断言 control+taxonomy+lessons 三 tab 面板小标题）改为逐 tab：
```tsx
it("一体化迁移：总控/Prompt/状态机/字典/教训面板小标题在各自 tab 渲染", () => {
  render(<SystemStrategyFeature />);
  // control tab（默认）
  expect(screen.getByText("系统总控 Prompt")).toBeInTheDocument();
  // taxonomy tab
  selectTab("标签与状态");
  expect(screen.getByText("状态机动作策略灰度")).toBeInTheDocument();
  expect(screen.getByText("双层标签字典灰度")).toBeInTheDocument();
  // lessons tab
  selectTab("经验教训");
  expect(screen.getByText("跨用户教训归纳（14d 滑窗）")).toBeInTheDocument();
});
```

- [ ] **Step 3: 拆 `:97` 空态跨 tab 断言用例**

```tsx
it("一体化迁移：各 tab 空态渲染，重置 Prompt Pack 按钮在总控 tab 可见", async () => {
  render(<SystemStrategyFeature />);
  // control tab
  expect(screen.getByText("重置系统提示词包 v2")).toBeInTheDocument();
  // taxonomy tab 空态
  selectTab("标签与状态");
  await waitFor(() => {
    expect(screen.getByText("暂无状态策略")).toBeInTheDocument();
  });
  expect(screen.getByText("暂无字典条目")).toBeInTheDocument();
  // lessons tab 空态
  selectTab("经验教训");
  expect(screen.getByText("暂无教训聚合（窗口内无命中样本）")).toBeInTheDocument();
});
```

- [ ] **Step 4: TaxonomiesAdmin 用例组切「标签与状态」tab**

`describe("TaxonomiesAdmin 新增条目")` / `describe("TaxonomiesAdmin 编辑与废弃恢复")` / `describe("TaxonomiesAdmin 边界")` 内每个 `render(<SystemStrategyFeature />);` 后紧跟一行 `selectTab("标签与状态");`。例（`:124` 用例）：
```tsx
    render(<SystemStrategyFeature />);
    selectTab("标签与状态");
    fireEvent.click(await screen.findByText("新增条目"));
```
对该文件中所有 TaxonomiesAdmin describe 的每个 it 都加此前置（共 render 出现处：`:124 :174 :187 :195 :208 :235 :254 :264`——逐一在 render 后加 selectTab("标签与状态")）。删除 `:126` 那句已过时注释「一次渲染全部面板，无需切 tab」。

- [ ] **Step 5: DomainProfilePanel 用例组切「行业配置」tab**

`describe("DomainProfile ... D10")`（`:307`）、`describe("DomainProfile completeness ... D11")`（`:357`）内 `render(<SystemStrategyFeature />);` 后加 `selectTab("行业配置");`。

- [ ] **Step 6: taxonomyFlags.test.tsx 切 tab**

顶部加同款 `selectTab` 辅助（或直接内联 click）。`:42`、`:62` 两处 `render(<SystemStrategyFeature />);` 后加 `selectTab("标签与状态");`（这两个用例断言 TaxonomiesAdmin 的终态/再激活 flag）。

- [ ] **Step 7: domainProfileVersions.test.tsx 切 tab**

`:59`、`:68` 两处 `render(<SystemStrategyFeature />);` 后加 `selectTab("行业配置");`（用例断言 DomainProfilePanel 的 ActiveVersionsBar）。顶部加 selectTab 辅助或内联。

- [ ] **Step 8: promptConfirm.test.tsx 确认默认 tab**

`:57`、`:85` 断言 DomainPromptPanel 的 prompt 保存/发布二次确认——该面板在默认 `control` tab，**无需切 tab**。仅确认：render 后直接断言即可。若这两个用例的目标元素在 control tab 首屏可见则不改；若因组件结构变化找不到，再加 `selectTab("总控与 Prompt")`（幂等，点默认 tab 无害）。本步先不改，留到 Step 10 跑测试时按结果决定。

- [ ] **Step 9: 运行测试验证“预期失败”**

Run: `cd frontend && npx vitest run src/__tests__/features/system-strategy/`
Expected: 多个用例 FAIL——报错形如 `Unable to find an accessible element with the role "button" and name "标签与状态"`（tab 按钮尚不存在，Task 2 才实现）。这证明测试确实依赖新 tab 结构。

- [ ] **Step 10: 不提交，进入 Task 2**

测试红是预期（RED），等 Task 2 组件实现后统一转绿再提交。

---

### Task 2: SystemStrategyInner tab 化（转绿）

**Files:**
- Modify: `frontend/src/features/system-strategy/index.tsx:3`（图标 import）、`:2478-2596`（`SystemStrategyInner` body）
- Modify（按需）: `frontend/src/features/system-strategy/SystemStrategy.module.css`

**Interfaces:**
- Consumes: Task 1 约定的 tab 中文标签文本（作为 button 文本）。
- Produces: 页面渲染 4 个 tab 按钮，点击切换只渲染对应组面板。

- [ ] **Step 1: 补 lucide 图标 import**

`index.tsx:3` 现为 `import { Settings2, Inbox } from "lucide-react";`。改为补入 4 个 tab 图标：
```tsx
import { Settings2, Inbox, SlidersHorizontal, Tags, Building2, GraduationCap } from "lucide-react";
```
（`SlidersHorizontal`=总控与 Prompt，`Tags`=标签与状态，`Building2`=行业配置，`GraduationCap`=经验教训。均为 lucide-react 现有导出。）

- [ ] **Step 2: 在 SystemStrategyInner 内定义 tab 常量与 state**

在 `function SystemStrategyInner() {` 内、`const busy = ...` 之后加：
```tsx
  type StrategyTab = "control" | "taxonomy" | "profile" | "lessons";
  const STRATEGY_TABS: { key: StrategyTab; label: string; Icon: typeof Settings2 }[] = [
    { key: "control", label: "总控与 Prompt", Icon: SlidersHorizontal },
    { key: "taxonomy", label: "标签与状态", Icon: Tags },
    { key: "profile", label: "行业配置", Icon: Building2 },
    { key: "lessons", label: "经验教训", Icon: GraduationCap },
  ];
  const [tab, setTab] = useState<StrategyTab>("control");
```
确认 `useState` 已 import（文件顶部若无 `import { useState } from "react"` 则补；`SystemStrategyInner` 已用 useEffect，通常 react hooks 已 import——先 grep 确认，缺才补）。

- [ ] **Step 3: 在 `<div className={styles.page}>` 内首行插入 tab bar**

紧跟 `<div className={styles.page}>` 之后：
```tsx
      <div className={styles.profileTabBar} style={{ marginBottom: 4 }}>
        {STRATEGY_TABS.map((t) => (
          <button
            key={t.key}
            type="button"
            className={`${styles.profileTab} ${tab === t.key ? styles.profileTabActive : ""}`}
            onClick={() => setTab(t.key)}
          >
            <t.Icon size={15} style={{ marginRight: 6, verticalAlign: "-2px" }} />
            {t.label}
          </button>
        ))}
      </div>
```

- [ ] **Step 4: 把系统总控 section + DomainPromptPanel 包进 control tab**

将 `<section className={styles.panel}>...系统总控...</section>`（`:2529` 起到其 `</section>`）与紧随的 `<DomainPromptPanel .../>`（`:2565`）一起包进：
```tsx
      {tab === "control" && (
        <>
          {/* 系统总控 section 原样 */}
          {/* <DomainPromptPanel .../> 原样 */}
        </>
      )}
```

- [ ] **Step 5: 把三个标签/状态面板包进 taxonomy tab**

```tsx
      {tab === "taxonomy" && (
        <>
          <StatePolicyAdmin busy={busy} />
          <TaxonomiesAdmin busy={busy} />
          <TaxonomyCandidatesAdmin busy={busy} />
        </>
      )}
```

- [ ] **Step 6: profile / lessons tab 各包一个面板**

```tsx
      {tab === "profile" && <DomainProfilePanel busy={busy} />}
      {tab === "lessons" && <LessonsLearnedAdmin busy={busy} />}
```
（注意源码原顺序 lessons 在 profile 之前——重排为按 tab 分组渲染，与 STRATEGY_TABS 顺序无强绑定，条件渲染顺序不影响。）

- [ ] **Step 7: 运行 system-strategy 测试验证转绿**

Run: `cd frontend && npx vitest run src/__tests__/features/system-strategy/`
Expected: 全部 PASS。若 promptConfirm 用例失败（找不到目标元素），按 Task 1 Step 8 加 `selectTab("总控与 Prompt")` 前置后再跑。

- [ ] **Step 8: tsc 类型检查**

Run: `cd frontend && npx tsc --noEmit`
Expected: 0 error。

- [ ] **Step 9: 全量 vitest**

Run: `cd frontend && npx vitest run`
Expected: 全绿（≥459 passed；不低于改前基线）。

- [ ] **Step 10: no-human-takeover lint**

Run: `bash scripts/check-no-human-takeover.sh`
Expected: 0 violations。

- [ ] **Step 11: 提交（测试+组件一起）**

```bash
git add frontend/src/features/system-strategy/index.tsx frontend/src/features/system-strategy/SystemStrategy.module.css frontend/src/__tests__/features/system-strategy/
git commit -m "fix(frontend): 系统策略频道改 4-tab 分区,消除页面无限长"
```

---

### Task 3: 人工核对 + 部署 117

- [ ] **Step 1: 起 dev server 人工核对（若本地可跑）或 build 静态核对**

Run: `cd frontend && npm run build`
Expected: 构建成功（`tsc && vite build` 全过）。人工检查（若起 dev）：4 tab 可切换、每 tab 只显示该组面板、页面不再无限长、默认落「总控与 Prompt」。

- [ ] **Step 2: 推送开 PR**

```bash
git push -u origin <branch>
gh pr create --base main --title "fix(frontend): 系统策略频道 4-tab 分区消除无限长" --body "..."
```

- [ ] **Step 3: CI 前端契约门绿后合并**

前端 PR，paths-filter 跳过后端 job，只跑「前端契约对账 (tsc + vitest)」。绿 → `gh pr merge --squash --delete-branch`。

- [ ] **Step 4: 部署 117 重建前端**

拉最新 main（ff）→ `frontend && npm run build`（dist 被 gitignore，必须重建）→ 无需重启（ServeDir 运行时读 dist）→ curl 核对入口 hash 已切新产物、页面可访问。

---

## Self-Review

- **Spec 覆盖**：4 组分区（Task 2 S4-6）✓；复用现有 CSS 类（Task 2 S3 用 `.profileTabBar/.profileTab/.profileTabActive`）✓；测试同步 5 文件（Task 1）✓；眉标保留（未动眉标）✓；不改面板内部（Task 2 只包裹不改 props）✓；三门（Task 2 S8-10）✓；部署（Task 3）✓。
- **占位符扫描**：无 TBD；每步有确切代码/命令/预期。promptConfirm Step 8 明确“先不改、按 Step 10 结果决定”是有条件的确定分支，非占位。
- **类型一致**：tab key `control/taxonomy/profile/lessons` 与标签文本在 Task 1（测试 selectTab 参数）与 Task 2（STRATEGY_TABS）逐字一致 ✓。图标名均 lucide-react 现有导出（实现时若某名不存在则换同类现有图标，不阻断）。
- **风险点**：Task 1 Step 4 列的 render 行号（`:124 :174...`）基于当前文件，实现时以“每个 TaxonomiesAdmin describe 的每个 it 的 render 后加 selectTab”为准，不依赖行号精确。

# 标签可信度改造 · 子计划 5：前端走势图 + 三层标签展示 + 人格画像 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 user-ops 联系人详情区新增四块前端 UI——(1) 三层标签分区展示（人工层可编辑 / AI 确信层只读带证据 / 贝叶斯评估层）；(2) `manual_tags` 运营录入表单（接 PUT /contacts/:id/manual-tags）；(3) 贝叶斯置信度走势图（手写 SVG）；(4) 大五 OCEAN 人格画像 + 演化图。

**Architecture:** 新建独立组件 + 各自 `.module.css`（避开 legacy.tsx 全局 className 的 tree-shake 坑），由 `UserOperationCockpit`（legacy.tsx）挂载。数据来自 `useContactStore.selected`（已含新字段，需扩 TS 类型）。走势图无图表库依赖，手写 SVG 折线。遵守 `docs/frontend-design-system.md` + `tokens.css`。

**Tech Stack:** React 19 + TypeScript + Vite + Zustand。无图表库（手写 SVG）。lucide-react 图标。CSS Modules。

## Global Constraints

- 前端验证：`cd frontend && npm run build`（tsc + vite）+ `npm run test`（vitest）本地可跑。
- **设计系统**：遵守 `docs/frontend-design-system.md`——4 级层级、颜色纪律（蓝仅主操作 / 紫仅 AI 身份）、字号、表单规范。真实 token 在 `src/components/ui/tokens.css`。
- **CSS module tree-shake 坑**：新组件用 `.module.css` + `import styles` + `className={styles.x}`（有引用即安全）；**不要**裸 `import "./x.module.css"`（会被摇掉）。
- **camelCase**：前端全 camelCase，新字段 `manualTags`/`confirmedTags`/`bayesianSignals`/`personalityProfile`（后端序列化层已转）。
- **no-human-takeover**：UI 文案避禁用词。人工层用"运营录入/运营确认标签"，不写"人工接管"。CI lint 扫 frontend/src 新增行。
- **AI 身份色**：AI 确信层 / 贝叶斯 / 人格用紫色系（AI 身份）；人工层用中性/主操作色。
- 提交需用户显式批准；精确 `git add`。

## 设计来源

`docs/superpowers/specs/2026-06-23-tag-trust-two-layer-design.md` —— "前端：置信度走势图 + 三层标签" 节（4 项）。

## 依赖

- **子计划 1-4 后端已完成**：`Contact` 含 manualTags/confirmedTags/bayesianSignals/personalityProfile；PUT /contacts/:id/manual-tags 端点就绪；ApiContact 投影这些字段。

## 现状核实（subagent 已核实，事实基线）

- 联系人详情根组件 `UserOperationCockpit`：`frontend/src/features/user-ops/legacy.tsx`（约 :130 起，2147 行）。画像/记忆展示 :240-313；`MemoryCardSummary`（:569）；运营编辑表单 :426-442（relationshipType select + onSaveRelationshipType）。
- **tags 当前在 user-ops 零渲染**（三层标签是全新 UI）。
- TS 类型：`frontend/src/types/index.ts`，`Contact`（:48-84，`tags: string[]` :62、`agentProfile?` :58、`domainAttributes?` :63）、`AgentProfile`（:41-46）。
- **无图表库**：`package.json` 仅 react/react-dom/zustand/lucide-react。走势图**手写 SVG**。
- API：`src/lib/api.ts:52-115`，`api.put<T>(url, body)`，URL 含 `/api` 前缀。
- store：选中 contact 在 `useContactStore.getState().selected`；详情加载 `userOpsStore.loadMessages(contact)`（:320-348 并行 5 接口）；写 profile 范例 `saveRelationshipType`（:516-536）。
- CSS：`tokens.css` 存在；`.module.css` 范例 `src/features/operations/index.tsx:8` `import styles from "./Operations.module.css"`；**legacy.tsx 用全局 styles.css 字符串类名、无 css import**（:3 注释说明）。

---

## Task 1：扩 TS 类型 + manual_tags store action

**Files:**
- Modify: `frontend/src/types/index.ts:41-84`（Contact + 新结构类型）
- Modify: `frontend/src/stores/userOpsStore.ts`（加 `saveManualTags` action）
- Test: `frontend/src/__tests__/`（store action 调用断言，仿现有 store 测试）

**Interfaces:**
- Produces: TS 类型 `Evidence`/`ConfirmedTag`/`BayesianSignal`/`BayesianPoint`/`PersonalityProfile`/`PersonalityFacet`/`PersonalitySnapshot`；Contact 加四字段；`saveManualTags(tags: string[])` action。

- [ ] **Step 1: 加 TS 类型**

`types/index.ts` 加（camelCase，对齐后端序列化）：
```typescript
export type Evidence = { turn: number; msgId: string };
export type ConfirmedTag = { value: string; evidences: Evidence[]; confirmedAt: string; confirmedBy: string };
export type BayesianPoint = { turn: number; value: string; confidence: number; valueChanged: boolean; confidenceChanged: boolean; reason?: string };
export type BayesianSignal = { dimension: string; currentValue: string; currentConfidence: number; locked: boolean; history: BayesianPoint[] };
export type PersonalityFacet = { score: number; confidence: number; evidenceRefs: Evidence[] };
export type PersonalitySnapshot = { consolidatedAt: string; scores: number[]; confidences: number[] };
export type PersonalityProfile = {
  openness: PersonalityFacet; conscientiousness: PersonalityFacet; extraversion: PersonalityFacet;
  agreeableness: PersonalityFacet; neuroticism: PersonalityFacet; updatedAt: string; snapshots: PersonalitySnapshot[];
};
```
`Contact`（:48-84）加：
```typescript
  manualTags?: string[];
  confirmedTags?: ConfirmedTag[];
  bayesianSignals?: BayesianSignal[];
  personalityProfile?: PersonalityProfile;
```
（用 optional `?`：后端可能不投影/为空，前端容错。）

- [ ] **Step 2: 加 saveManualTags action**

`userOpsStore.ts` 仿 `saveRelationshipType`（:516-536）：
```typescript
saveManualTags: async (tags: string[]) => {
  const selected = useContactStore.getState().selected;
  if (!selected) return;
  await api.put(`/api/contacts/${selected.id}/manual-tags`, { tags });
  await get().refreshContacts?.(); // 对齐现有 refresh 方式
},
```
（确认 store interface 类型定义处也加 `saveManualTags` 签名。）

- [ ] **Step 3: 写 store action 测试**

仿 `frontend/src/__tests__/` 现有 store 测试，mock api.put，断言 `saveManualTags(["vip"])` 调 `api.put` 且 URL/body 正确。

- [ ] **Step 4: 构建验证**

Run: `cd frontend && npm run build`
Expected: tsc 0 error。
Run: `cd frontend && npm run test`
Expected: 新测试 pass + 既有不回归。

- [ ] **Step 5: 提交**

```bash
git add frontend/src/types/index.ts frontend/src/stores/userOpsStore.ts frontend/src/__tests__/
git commit -m "feat(tag-trust-fe): TS types for trust fields + saveManualTags store action (子计划5 Task1)"
```

---

## Task 2：三层标签展示 + manual_tags 录入组件

**Files:**
- Create: `frontend/src/features/user-ops/TagTrustPanel.tsx`（新独立组件）
- Create: `frontend/src/features/user-ops/TagTrustPanel.module.css`
- Modify: `frontend/src/features/user-ops/legacy.tsx`（在画像区 :240-313 附近挂载 `<TagTrustPanel contact={selected} onSaveManualTags={...} />`）
- Test: `frontend/src/__tests__/features/user-ops/tagTrustPanel.test.tsx`

**Interfaces:**
- Consumes: Task 1 类型 + `saveManualTags`。
- Produces: 组件展示三层：人工层（chip + 可编辑输入，自由文本逗号分隔）、AI 确信层（chip 只读，hover/展开显示证据条数）、贝叶斯层（占位，Task 3 填走势图）。

- [ ] **Step 1: 写组件测试（先定行为）**

```tsx
// tagTrustPanel.test.tsx
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import TagTrustPanel from "../../../features/user-ops/TagTrustPanel";

vi.mock("../../../features/user-ops/TagTrustPanel.module.css", () => ({
  default: new Proxy({}, { get: (_t, k) => String(k) }),
}));

const baseContact = {
  id: "c1", manualTags: ["VIP"], confirmedTags: [{ value: "价格敏感", evidences: [{turn:0,msgId:"x"}], confirmedAt:"", confirmedBy:"consolidation" }],
  bayesianSignals: [],
} as any;

describe("TagTrustPanel", () => {
  it("三层分区都渲染，人工层与 AI 层来源可区分", () => {
    render(<TagTrustPanel contact={baseContact} onSaveManualTags={vi.fn()} />);
    expect(screen.getByText("VIP")).toBeInTheDocument();
    expect(screen.getByText("价格敏感")).toBeInTheDocument();
    expect(screen.getByText(/运营录入/)).toBeInTheDocument();
    expect(screen.getByText(/AI 判断/)).toBeInTheDocument();
  });

  it("AI 确信标签显示证据条数", () => {
    render(<TagTrustPanel contact={baseContact} onSaveManualTags={vi.fn()} />);
    expect(screen.getByText(/1 条证据/)).toBeInTheDocument();
  });

  it("编辑人工标签保存调 onSaveManualTags", async () => {
    const onSave = vi.fn();
    render(<TagTrustPanel contact={baseContact} onSaveManualTags={onSave} />);
    fireEvent.click(screen.getByText("编辑"));
    fireEvent.change(screen.getByPlaceholderText(/逗号分隔/), { target: { value: "VIP, 老客户" } });
    fireEvent.click(screen.getByText("保存"));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith(["VIP", "老客户"]));
  });
});
```

- [ ] **Step 2: 运行确认失败**

Run: `cd frontend && npm run test tagTrustPanel`
Expected: FAIL（组件不存在）。

- [ ] **Step 3: 写组件 + module.css**

`TagTrustPanel.tsx`：三个分区 `<section>`，人工层带"编辑"切换 + 逗号分隔输入（split `/[,，]/` map trim filter，复用项目惯例）；AI 确信层 chip + `{tag.evidences.length} 条证据`；贝叶斯层留 `<BayesianTrendChart>` 占位（Task 3）。文案标注来源："运营录入（权威）" / "AI 判断（可能调整）"。
`.module.css`：用 tokens.css 变量，AI 层 chip 用紫色系（AI 身份），人工层用中性色。遵守 design-system 字号/间距。

- [ ] **Step 4: 挂载进 legacy.tsx + 构建**

在 `UserOperationCockpit` 画像区（legacy.tsx :240-313 附近）挂 `<TagTrustPanel contact={selected} onSaveManualTags={saveManualTags} />`（从 store 取 action）。
Run: `cd frontend && npm run test tagTrustPanel` → pass。
Run: `cd frontend && npm run build` → 0 error。

- [ ] **Step 5: 提交**

```bash
git add frontend/src/features/user-ops/TagTrustPanel.tsx frontend/src/features/user-ops/TagTrustPanel.module.css frontend/src/features/user-ops/legacy.tsx frontend/src/__tests__/features/user-ops/tagTrustPanel.test.tsx
git commit -m "feat(tag-trust-fe): three-layer tag panel + manual tags editor (子计划5 Task2)"
```

---

## Task 3：贝叶斯置信度走势图（手写 SVG）

**Files:**
- Create: `frontend/src/features/user-ops/BayesianTrendChart.tsx`
- Create: `frontend/src/features/user-ops/BayesianTrendChart.module.css`
- Modify: `frontend/src/features/user-ops/TagTrustPanel.tsx`（贝叶斯分区填入图表）
- Test: `frontend/src/__tests__/features/user-ops/bayesianTrendChart.test.tsx`

**Interfaces:**
- Consumes: `BayesianSignal[]`（每个 locked 槽一条线，消费 `history`）。
- Produces: 手写 SVG 折线图，x=轮次、y=置信度 0~1，每个已占槽维度一条线 + 图例。

- [ ] **Step 1: 写测试**

```tsx
const signals = [{
  dimension: "价格敏感度", currentValue: "高", currentConfidence: 0.7, locked: true,
  history: [
    { turn: 1, value: "高", confidence: 0.4, valueChanged: false, confidenceChanged: true },
    { turn: 2, value: "高", confidence: 0.7, valueChanged: false, confidenceChanged: true },
  ],
}] as any;

it("每个 locked 维度渲染一条 polyline + 图例", () => {
  const { container } = render(<BayesianTrendChart signals={signals} />);
  expect(container.querySelectorAll("polyline").length).toBe(1);
  expect(screen.getByText("价格敏感度")).toBeInTheDocument();
});

it("未占槽(locked=false)维度不画线", () => {
  const { container } = render(<BayesianTrendChart signals={[{...signals[0], locked:false}]} />);
  expect(container.querySelectorAll("polyline").length).toBe(0);
});

it("无数据显示空态", () => {
  render(<BayesianTrendChart signals={[]} />);
  expect(screen.getByText(/暂无评估维度/)).toBeInTheDocument();
});
```

- [ ] **Step 2: 运行确认失败**

Run: `cd frontend && npm run test bayesianTrendChart`
Expected: FAIL。

- [ ] **Step 3: 写 SVG 折线图实现**

纯 SVG：固定 viewBox（如 `0 0 320 160`），把每个 locked signal 的 history 映射成 `<polyline points="...">`，y = `(1 - confidence) * height`，x = `turn` 归一到宽度。多维度多色（从 tokens 取一组色）。加 y 轴 0/0.5/1 刻度线、图例（维度名 + 当前值 + 当前置信度）。空态文案"暂无评估维度（需多轮强证据才占槽）"。不引图表库。

> 颜色：贝叶斯是 AI 评估层，主色用紫系；多线时用紫系不同明度或一组可区分色（确认 tokens.css 有哪些可用色变量，无则在 module.css 定义局部色板）。

- [ ] **Step 4: 填入 TagTrustPanel + 构建**

TagTrustPanel 贝叶斯分区渲染 `<BayesianTrendChart signals={contact.bayesianSignals ?? []} />`。
Run: `cd frontend && npm run test bayesianTrendChart` → pass。
Run: `cd frontend && npm run build` → 0 error。

- [ ] **Step 5: 提交**

```bash
git add frontend/src/features/user-ops/BayesianTrendChart.tsx frontend/src/features/user-ops/BayesianTrendChart.module.css frontend/src/features/user-ops/TagTrustPanel.tsx frontend/src/__tests__/features/user-ops/bayesianTrendChart.test.tsx
git commit -m "feat(tag-trust-fe): hand-rolled SVG bayesian confidence trend chart (子计划5 Task3)"
```

---

## Task 4：大五 OCEAN 人格画像 + 演化图

**Files:**
- Create: `frontend/src/features/user-ops/PersonalityPanel.tsx`
- Create: `frontend/src/features/user-ops/PersonalityPanel.module.css`
- Modify: `frontend/src/features/user-ops/legacy.tsx`（画像区挂载）
- Test: `frontend/src/__tests__/features/user-ops/personalityPanel.test.tsx`

**Interfaces:**
- Consumes: `PersonalityProfile`（五维当前分 + confidence + snapshots）。
- Produces: 五维条形/雷达展示（当前分 + 置信度，低置信视觉弱化）+ 演化折线（消费 snapshots，复用 Task 3 的 SVG 折线思路，x=压缩周期）。

- [ ] **Step 1: 写测试**

```tsx
const profile = {
  openness: { score: 0.7, confidence: 0.4, evidenceRefs: [{turn:0,msgId:"x"}] },
  conscientiousness: { score: 0.5, confidence: 0.0, evidenceRefs: [] },
  extraversion: { score: 0.6, confidence: 0.3, evidenceRefs: [] },
  agreeableness: { score: 0.8, confidence: 0.5, evidenceRefs: [] },
  neuroticism: { score: 0.3, confidence: 0.2, evidenceRefs: [] },
  updatedAt: "", snapshots: [],
} as any;

it("五维都渲染，低置信维度标注存疑", () => {
  render(<PersonalityPanel profile={profile} />);
  ["开放性","尽责性","外向性","宜人性","神经质"].forEach(n =>
    expect(screen.getByText(n)).toBeInTheDocument());
  // conscientiousness confidence=0 → 低置信视觉/文案
  expect(screen.getByText(/证据不足|存疑|低置信/)).toBeInTheDocument();
});

it("无 profile 显示空态", () => {
  render(<PersonalityPanel profile={undefined} />);
  expect(screen.getByText(/暂无人格分析/)).toBeInTheDocument();
});
```

- [ ] **Step 2: 运行确认失败**

Run: `cd frontend && npm run test personalityPanel`
Expected: FAIL。

- [ ] **Step 3: 写实现**

五维映射中文名（开放性/尽责性/外向性/宜人性/神经质），每维一条横向 bar（宽=score），低 confidence（如 <0.3）的维度灰化 + 标"证据不足"。下方演化区:若 `snapshots.length>=2` 画 SVG 折线（五维五条线，x=snapshot 序号），否则提示"演化需多次归并后呈现"。科学定位文案:"基于大五人格（OCEAN），从对话行为推断，仅供参考"。

- [ ] **Step 4: 挂载 + 构建**

legacy.tsx 画像区挂 `<PersonalityPanel profile={selected.personalityProfile} />`。
Run: `cd frontend && npm run test personalityPanel` → pass。
Run: `cd frontend && npm run build` → 0 error。

- [ ] **Step 5: 提交**

```bash
git add frontend/src/features/user-ops/PersonalityPanel.tsx frontend/src/features/user-ops/PersonalityPanel.module.css frontend/src/features/user-ops/legacy.tsx frontend/src/__tests__/features/user-ops/personalityPanel.test.tsx
git commit -m "feat(tag-trust-fe): OCEAN personality panel + evolution chart (子计划5 Task4)"
```

---

## Task 5：浏览器实跑验证（golden path）

**Files:** 无（验证任务）

- [ ] **Step 1: 起前端 dev server**

Run: `cd frontend && npm run dev`（vite，代理 /api → :8080）。后端需另起 `cargo run`（或用现有 mock 数据）。

- [ ] **Step 2: 浏览器走查**

打开 user-ops 频道，选一个 contact，确认：
- 三层标签分区都显示，人工/AI 来源视觉可区分。
- 编辑人工标签 → 保存 → 刷新后保留。
- 贝叶斯走势图渲染（若该 contact 无 locked 槽，显示空态文案）。
- 人格面板渲染（无数据显示空态）。
- 整体符合 design-system（颜色/层级/间距）。

> 若后端数据未就绪（子计划 1-4 未部署），用 mock contact 数据注入 store 验证渲染。**明确报告：能测渲染，业务数据流需后端联调**。

- [ ] **Step 3: 记录结果**

截图或文字记录 golden path 结果。UI 测试不通过的项回到对应 Task 修。

> 本任务不提交代码，是交付前的人工/浏览器验证关。

---

## Self-Review（写计划者自检）

**Spec 覆盖：**
- 置信度走势图（消费 history）→ Task 3 ✓
- 三层标签分区展示 → Task 2 ✓
- manual_tags 录入 UI → Task 1（action）+ Task 2（表单）✓
- 大五人格画像 + 演化图（消费 snapshots）→ Task 4 ✓

**占位符扫描：** Task 3/4 的 SVG 映射用散文描述（viewBox、polyline points 公式）+ 测试钉死行为（polyline 数量、空态文案），非占位——SVG 具体坐标交实现者按测试通过即可。Task 5 是验证任务无代码。

**类型一致：** TS 类型（Task 1）camelCase 与后端序列化一致；`BayesianSignal.history`/`PersonalityProfile.snapshots` 在 Task 3/4 消费字段名一致；`onSaveManualTags(tags: string[])` 在 Task 1 action 与 Task 2 组件 prop 一致 ✓。

**设计系统合规：** 每个新组件独立 .module.css（避 legacy.tsx tree-shake 坑）；AI 层/贝叶斯/人格用紫系（AI 身份色纪律）；文案避禁用词（"运营录入"非"人工接管"）✓。

**需实现期核实（已标注）：** userOpsStore 是否有 refreshContacts、tokens.css 可用色变量清单、legacy.tsx 挂载点的 props 传递方式、vitest CSS module mock 惯例（systemStrategy.test.tsx:12 有范例）。

---

## 全 5 子计划完成后

5 个子计划全部实现 + 各自 review 通过后，用 superpowers:subagent-driven-development 的最终 whole-branch review 做一次跨子计划终审（重点验"永不驱动"铁律、三层隔离、证据 fail-closed、基线不回归），再用 superpowers:finishing-a-development-branch 收尾。

# 知识库可信度治理驾驶舱 · 前端实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让前端知识库 UI 显形后端「可信度治理」新业务逻辑(answeringMode + 5 维认知矩阵、D2 闸、承诺背书、对话补库、auto-verify、富字段),并用大白话基调让小白运营可用。

**Architecture:** 升级现有 `StewardMode`(features/knowledge/index.tsx)——新增 `cockpit` pane 作默认首屏(治理驾驶舱主屏),`review` pane 升级为审核+对话双栏,新增 `autoVerify` pane。先修数据契约(schema 漂移)作地基,再逐屏建 UI。纯前端 + 视觉复用 components/ui。

**Tech Stack:** React 19 + TypeScript + Vite;CSS Modules;Zustand;Vitest + jsdom + @testing-library;lucide-react 图标。设计 token 全在 `frontend/src/components/ui/tokens.css`。

**设计依据:** `docs/superpowers/specs/2026-06-07-knowledge-trust-cockpit-frontend-design.md` + mockup `.superpowers/brainstorm/1455-1780766245/content/`(cockpit-final / review-chat-v2 / single-action / plain-language / auto-verify)。

**测试约定(已核实):** 测试只放 `frontend/src/__tests__/**/*.test.{ts,tsx}`(vitest.config.ts include glob);setup 在 `src/__tests__/setup.ts`(挂 jest-dom)。CSS module 需 `src/vite-env.d.ts` 的 `declare module "*.module.css"`。

**运行命令(在 `E:/yw/wa-kb-cockpit/frontend`):**
- 测试单文件:`npm run test -- src/__tests__/features/<name>.test.tsx`
- 全量构建:`npm run build`(tsc + vite,严格类型)

**关键约定(踩坑后确认,来自 [[project_frontend_refactor]]):**
- 新组件用 `<button>` 必须在自己 `.module.css` 里重置(`background/border/box-shadow/min-height/justify-content:flex-start` + 覆盖 hover),否则被全局 `styles.css:71-118` 裸 button 规则污染(蓝底/居中)。
- 页头(eyebrow/title)由上层渲染,feature 内不重复写大标题。
- 状态色只用 tokens.css 的 `var(--color-*)`,禁止硬编码。

---

## 阶段一:数据契约修复(地基)

修 schema 漂移——前端类型停在旧结构,拿旧 key 读后端新响应导致静默吃数据。这阶段是纯类型 + 解析层,不动 UI 外观,但能立刻修复 completeness 黑洞。

### Task 1: CompletenessView 类型对齐后端真实响应

后端 `GET /api/operation-knowledge/completeness` 真实返回(knowledge.rs:3367-3378):`totalChunks/verifiedChunks/anchoredChunks/evidenceChunks/needsReviewChunks/pendingReview/answeringMode/summary/coverage/gaps`。其中 `coverage` 是 5 维对象,每维 `{verifiedFact,methodologyOnly,pendingDraft,state}`。前端现有 `CompletenessView`(index.tsx:4264-4267)只认 `perWikiType/overall`——全错。

**Files:**
- Create: `frontend/src/features/knowledge/trustTypes.ts`(可信度治理相关共享类型,新建)
- Modify: `frontend/src/features/knowledge/index.tsx:4264-4267`(替换旧 interface 为 import)
- Test: `frontend/src/__tests__/features/trustTypes.test.ts`

- [ ] **Step 1: 写失败测试**

新建 `frontend/src/__tests__/features/trustTypes.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { parseCompleteness, type CompletenessView, type CoverageDimension } from "../../features/knowledge/trustTypes";

describe("parseCompleteness", () => {
  it("解析后端真实响应的 answeringMode + 5 维 coverage", () => {
    const raw = {
      totalChunks: 40, verifiedChunks: 12, anchoredChunks: 10,
      evidenceChunks: 3, needsReviewChunks: 2,
      answeringMode: "product_safe", summary: "可安全讲产品",
      coverage: {
        capability: { verifiedFact: true, methodologyOnly: false, pendingDraft: false, state: "verified" },
        pricing: { verifiedFact: false, methodologyOnly: false, pendingDraft: true, state: "draft" },
        caseEvidence: { verifiedFact: true, methodologyOnly: false, pendingDraft: false, state: "verified" },
        effectClaims: { verifiedFact: false, methodologyOnly: false, pendingDraft: false, state: "missing" },
        deliveryBoundary: { verifiedFact: false, methodologyOnly: true, pendingDraft: false, state: "methodology" },
      },
      gaps: ["效果数据维度无任何已验证知识"],
    };
    const v: CompletenessView = parseCompleteness(raw);
    expect(v.answeringMode).toBe("product_safe");
    expect(v.coverage.pricing.pendingDraft).toBe(true);
    expect(v.coverage.effectClaims.state).toBe("missing");
    expect(v.gaps).toHaveLength(1);
    expect(v.needsReviewChunks).toBe(2);
  });

  it("缺字段时降级为安全默认(空 coverage / relationship_only)", () => {
    const v = parseCompleteness({});
    expect(v.answeringMode).toBe("relationship_only");
    expect(v.coverage.capability.state).toBe("missing");
    expect(v.gaps).toEqual([]);
  });

  it("dimensionList 按固定顺序返回 5 维带中文名", () => {
    const v = parseCompleteness({});
    const dims: CoverageDimension[] = v.dimensionList;
    expect(dims.map((d) => d.key)).toEqual([
      "capability", "pricing", "caseEvidence", "effectClaims", "deliveryBoundary",
    ]);
    expect(dims[0].label).toBe("能力");
  });
});
```

- [ ] **Step 2: 运行测试确认失败**

Run: `npm run test -- src/__tests__/features/trustTypes.test.ts`
Expected: FAIL("Cannot find module '../../features/knowledge/trustTypes'")

- [ ] **Step 3: 写实现**

新建 `frontend/src/features/knowledge/trustTypes.ts`:

```ts
// 可信度治理:与后端 completeness / chunk 富字段对齐的类型 + 解析层。
// 后端真实响应见 src/routes/knowledge.rs:3367-3378(completeness)。

export type AnsweringMode = "relationship_only" | "product_safe" | "fully_supported";
export type CoverageState = "verified" | "methodology" | "draft" | "missing";

export interface CoverageFlags {
  verifiedFact: boolean;
  methodologyOnly: boolean;
  pendingDraft: boolean;
  state: CoverageState;
}

export interface CoverageDimension extends CoverageFlags {
  key: string;
  label: string; // 中文维度名
}

export interface CompletenessView {
  totalChunks: number;
  verifiedChunks: number;
  anchoredChunks: number;
  evidenceChunks: number;
  needsReviewChunks: number;
  answeringMode: AnsweringMode;
  summary: string;
  coverage: Record<DimKey, CoverageFlags>;
  gaps: string[];
  dimensionList: CoverageDimension[];
}

type DimKey = "capability" | "pricing" | "caseEvidence" | "effectClaims" | "deliveryBoundary";

const DIM_ORDER: { key: DimKey; label: string }[] = [
  { key: "capability", label: "能力" },
  { key: "pricing", label: "定价" },
  { key: "caseEvidence", label: "案例" },
  { key: "effectClaims", label: "效果数据" },
  { key: "deliveryBoundary", label: "交付边界" },
];

function flags(raw: unknown): CoverageFlags {
  const o = (raw ?? {}) as Record<string, unknown>;
  const verifiedFact = o.verifiedFact === true;
  const methodologyOnly = o.methodologyOnly === true;
  const pendingDraft = o.pendingDraft === true;
  const state: CoverageState =
    typeof o.state === "string" &&
    ["verified", "methodology", "draft", "missing"].includes(o.state)
      ? (o.state as CoverageState)
      : verifiedFact ? "verified"
      : methodologyOnly ? "methodology"
      : pendingDraft ? "draft"
      : "missing";
  return { verifiedFact, methodologyOnly, pendingDraft, state };
}

export function parseCompleteness(raw: unknown): CompletenessView {
  const o = (raw ?? {}) as Record<string, unknown>;
  const cov = (o.coverage ?? {}) as Record<string, unknown>;
  const coverage = Object.fromEntries(
    DIM_ORDER.map((d) => [d.key, flags(cov[d.key])])
  ) as Record<DimKey, CoverageFlags>;
  const mode = o.answeringMode;
  const answeringMode: AnsweringMode =
    mode === "product_safe" || mode === "fully_supported" ? mode : "relationship_only";
  return {
    totalChunks: Number(o.totalChunks ?? 0),
    verifiedChunks: Number(o.verifiedChunks ?? 0),
    anchoredChunks: Number(o.anchoredChunks ?? 0),
    evidenceChunks: Number(o.evidenceChunks ?? 0),
    needsReviewChunks: Number(o.needsReviewChunks ?? 0),
    answeringMode,
    summary: typeof o.summary === "string" ? o.summary : "",
    coverage,
    gaps: Array.isArray(o.gaps) ? o.gaps.filter((g): g is string => typeof g === "string") : [],
    dimensionList: DIM_ORDER.map((d) => ({ key: d.key, label: d.label, ...coverage[d.key] })),
  };
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `npm run test -- src/__tests__/features/trustTypes.test.ts`
Expected: PASS(3 个用例)

- [ ] **Step 5: 替换 index.tsx 旧 CompletenessView,接 parseCompleteness**

index.tsx:4264-4267 删掉旧 interface,改从 trustTypes import。ObservabilityDashboard 里 `setCompleteness(c as CompletenessView)`(约 4570)改成 `setCompleteness(parseCompleteness(c))`。顶部 import 加 `import { parseCompleteness, type CompletenessView } from "./trustTypes";`。

- [ ] **Step 6: 构建验证**

Run: `npm run build`
Expected: tsc 0 error(旧 `perWikiType` 引用若报错,一并改为读 `coverage`/`dimensionList`;ObservabilityDashboard 里渲染 completeness 的 JSX 暂时只显示 answeringMode + summary,coverage 渲染留给阶段二)

- [ ] **Step 7: Commit**

```bash
git add frontend/src/features/knowledge/trustTypes.ts frontend/src/__tests__/features/trustTypes.test.ts frontend/src/features/knowledge/index.tsx
git commit -m "feat(kb-frontend): CompletenessView 对齐后端 answeringMode+5维认知 schema"
```

### Task 2: IntegrityReportView 类型对齐

后端 `GET /api/operation-knowledge/integrity-report` 真实返回(knowledge.rs:1101-1151 区域)`item: { total, verified, needsReview, rejected, items[] }`。前端 `IntegrityReportView`(index.tsx:4269-4274)认 `needsReview/contested/sourceOrphan/total`——`contested/sourceOrphan` 后端不存在。

**Files:**
- Modify: `frontend/src/features/knowledge/trustTypes.ts`(加 IntegrityReportView + parseIntegrityReport)
- Modify: `frontend/src/features/knowledge/index.tsx:4269-4274`
- Test: `frontend/src/__tests__/features/trustTypes.test.ts`(append)

- [ ] **Step 1: append 失败测试**

在 trustTypes.test.ts 末尾追加:

```ts
import { parseIntegrityReport } from "../../features/knowledge/trustTypes";

describe("parseIntegrityReport", () => {
  it("读后端 item.{total,verified,needsReview,rejected}", () => {
    const v = parseIntegrityReport({ item: { total: 40, verified: 12, needsReview: 2, rejected: 1 } });
    expect(v.total).toBe(40);
    expect(v.verified).toBe(12);
    expect(v.needsReview).toBe(2);
    expect(v.rejected).toBe(1);
  });
  it("缺 item 时全 0", () => {
    const v = parseIntegrityReport({});
    expect(v.total).toBe(0);
    expect(v.verified).toBe(0);
  });
});
```

- [ ] **Step 2: 运行确认失败**

Run: `npm run test -- src/__tests__/features/trustTypes.test.ts`
Expected: FAIL("parseIntegrityReport is not a function")

- [ ] **Step 3: 实现**

trustTypes.ts 追加:

```ts
export interface IntegrityReportView {
  total: number;
  verified: number;
  needsReview: number;
  rejected: number;
}

export function parseIntegrityReport(raw: unknown): IntegrityReportView {
  const item = ((raw ?? {}) as Record<string, unknown>).item ?? {};
  const o = item as Record<string, unknown>;
  return {
    total: Number(o.total ?? 0),
    verified: Number(o.verified ?? 0),
    needsReview: Number(o.needsReview ?? 0),
    rejected: Number(o.rejected ?? 0),
  };
}
```

- [ ] **Step 4: 运行确认通过**

Run: `npm run test -- src/__tests__/features/trustTypes.test.ts`
Expected: PASS(5 个用例)

- [ ] **Step 5: 替换 index.tsx**

index.tsx:4269-4274 删旧 interface,import 改用 trustTypes。`setIntegrity(d as IntegrityReportView)` 改 `setIntegrity(parseIntegrityReport(d))`。删除所有读 `.contested`/`.sourceOrphan` 的 JSX(后端无此字段),改读 `.verified`/`.rejected`。

- [ ] **Step 6: 构建验证**

Run: `npm run build`
Expected: tsc 0 error

- [ ] **Step 7: Commit**

```bash
git add frontend/src/features/knowledge/trustTypes.ts frontend/src/__tests__/features/trustTypes.test.ts frontend/src/features/knowledge/index.tsx
git commit -m "feat(kb-frontend): IntegrityReportView 对齐后端 verified/rejected 字段"
```

### Task 3: ReviewChunkItem 补齐 chunk 富字段

后端 chunk JSON(knowledge.rs operation_knowledge_chunk_json,约 2266-2303)下发但前端 `ReviewChunkItem`(index.tsx:2477-2494)从不读的字段:`chunkType/provenance/validFrom/validTo/usageStats/dynamicConfidence/lockedFields/confidenceScore/distortionRisks`。本任务只扩类型(为阶段二 UI 铺路),不改 UI。

**Files:**
- Modify: `frontend/src/features/knowledge/index.tsx:2477-2494`(扩 ReviewChunkItem)
- Test: `frontend/src/__tests__/features/trustTypes.test.ts`(append,验证字段可选不破坏旧数据)

- [ ] **Step 1: append 失败测试**

```ts
import type { TrustChunkFields } from "../../features/knowledge/trustTypes";

describe("TrustChunkFields", () => {
  it("富字段全可选——旧数据(只有 id/title)仍合法", () => {
    const legacy: TrustChunkFields = {};
    expect(legacy.chunkType).toBeUndefined();
    const full: TrustChunkFields = {
      chunkType: "product_fact",
      distortionRisks: ["缺锚点已降级"],
      lockedFields: ["sourceQuote"],
      usageStats: { hitCount30d: 5, blockedCount30d: 1 },
      validFrom: "2026-01-01", validTo: null,
      dynamicConfidence: 0.72, confidenceScore: 8,
      provenance: { source: "ai", llmModelAlias: "mimo" },
    };
    expect(full.chunkType).toBe("product_fact");
    expect(full.usageStats?.hitCount30d).toBe(5);
  });
});
```

- [ ] **Step 2: 运行确认失败**

Run: `npm run test -- src/__tests__/features/trustTypes.test.ts`
Expected: FAIL("Cannot find ... TrustChunkFields")

- [ ] **Step 3: 实现——trustTypes.ts 加 TrustChunkFields**

```ts
export type ChunkType = "product_fact" | "style_template" | "negative_example" | "peer_case";

export interface ChunkUsageStats { hitCount30d?: number; blockedCount30d?: number; }
export interface ChunkProvenanceView { source?: string; llmModelAlias?: string | null; }

export interface TrustChunkFields {
  chunkType?: ChunkType | null;
  confidenceScore?: number | null;
  dynamicConfidence?: number | null;
  distortionRisks?: string[] | null;
  lockedFields?: string[] | null;
  validFrom?: string | null;
  validTo?: string | null;
  usageStats?: ChunkUsageStats | null;
  provenance?: ChunkProvenanceView | null;
}
```

- [ ] **Step 4: index.tsx:2494 处 ReviewChunkItem 加 `extends TrustChunkFields`**

将 `interface ReviewChunkItem {` 改为从 trustTypes import TrustChunkFields 并 `interface ReviewChunkItem extends TrustChunkFields {`(保留原有字段)。顶部 import 追加 `TrustChunkFields`。

- [ ] **Step 5: 运行测试 + 构建**

Run: `npm run test -- src/__tests__/features/trustTypes.test.ts && npm run build`
Expected: 测试 6 用例 PASS;tsc 0 error

- [ ] **Step 6: Commit**

```bash
git add frontend/src/features/knowledge/trustTypes.ts frontend/src/__tests__/features/trustTypes.test.ts frontend/src/features/knowledge/index.tsx
git commit -m "feat(kb-frontend): ReviewChunkItem 补 chunk 富字段类型(chunkType/usageStats/lockedFields 等)"
```

---

## 阶段二:治理驾驶舱主屏(cockpit pane)

把 mockup `cockpit-final.html` 落成真实组件,作 StewardMode 默认首屏。

### Task 4: AnsweringMode 极简仪表组件

**Files:**
- Create: `frontend/src/features/knowledge/cockpit/AnsweringModeGauge.tsx` + `.module.css`
- Test: `frontend/src/__tests__/features/AnsweringModeGauge.test.tsx`

- [ ] **Step 1: 失败测试**

```tsx
import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { AnsweringModeGauge } from "../../features/knowledge/cockpit/AnsweringModeGauge";

describe("AnsweringModeGauge", () => {
  it("product_safe 显示「可安全讲产品」+ 2/3 档", () => {
    render(<AnsweringModeGauge mode="product_safe" needsReviewChunks={2} summary="" />);
    expect(screen.getByText(/可安全讲产品/)).toBeInTheDocument();
    expect(screen.getByText(/2\s*\/\s*3/)).toBeInTheDocument();
  });
  it("有待审草稿时解读「为什么没到完全支撑」", () => {
    render(<AnsweringModeGauge mode="product_safe" needsReviewChunks={2} summary="" />);
    expect(screen.getByText(/待审/)).toBeInTheDocument();
  });
  it("fully_supported 显示「完全支撑」", () => {
    render(<AnsweringModeGauge mode="fully_supported" needsReviewChunks={0} summary="" />);
    expect(screen.getByText(/完全支撑/)).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 运行确认失败**

Run: `npm run test -- src/__tests__/features/AnsweringModeGauge.test.tsx`
Expected: FAIL(模块不存在)

- [ ] **Step 3: 实现组件**

`AnsweringModeGauge.tsx`——props `{ mode: AnsweringMode; needsReviewChunks: number; summary: string }`。三档映射:relationship_only→「仅关系维护」1/3、product_safe→「可安全讲产品」2/3、fully_supported→「完全支撑」3/3。呼吸蓝点(`.module.css` 用 `var(--color-scheduled)` + breathe 动画)、细进度条、一句话解读(needsReviewChunks>0 时显示「距完全支撑差一步:有 N 条待审草稿,有待审就绝不宣称完全支撑」)。文案/样式照 mockup `cockpit-final.html` 的 `.am` 块。CSS 用 tokens.css 变量,button 若有需重置。

- [ ] **Step 4: 运行确认通过**

Run: `npm run test -- src/__tests__/features/AnsweringModeGauge.test.tsx`
Expected: PASS(3 用例)

- [ ] **Step 5: Commit**

```bash
git add frontend/src/features/knowledge/cockpit/ frontend/src/__tests__/features/AnsweringModeGauge.test.tsx
git commit -m "feat(kb-frontend): AnsweringMode 极简仪表组件"
```

### Task 5: 5 维大白话裁决组件

**Files:**
- Create: `frontend/src/features/knowledge/cockpit/CoverageVerdict.tsx` + `.module.css`
- Test: `frontend/src/__tests__/features/CoverageVerdict.test.tsx`

- [ ] **Step 1: 失败测试**

```tsx
import { render, screen } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { CoverageVerdict } from "../../features/knowledge/cockpit/CoverageVerdict";
import { parseCompleteness } from "../../features/knowledge/trustTypes";

const view = parseCompleteness({
  answeringMode: "product_safe",
  coverage: {
    capability: { verifiedFact: true, state: "verified" },
    pricing: { pendingDraft: true, state: "draft" },
    caseEvidence: { verifiedFact: true, state: "verified" },
    effectClaims: { state: "missing" },
    deliveryBoundary: { methodologyOnly: true, state: "methodology" },
  },
});

describe("CoverageVerdict", () => {
  it("渲染 5 维,每维一行带中文名", () => {
    render(<CoverageVerdict view={view} onDrillDown={() => {}} />);
    ["能力","定价","案例","效果数据","交付边界"].forEach((n) =>
      expect(screen.getByText(n)).toBeInTheDocument());
  });
  it("effectClaims=missing 显示高风险大白话", () => {
    render(<CoverageVerdict view={view} onDrillDown={() => {}} />);
    expect(screen.getByText(/拦/)).toBeInTheDocument(); // 「会被…拦下」
  });
  it("点维度行触发 onDrillDown(维度 key)", () => {
    const fn = vi.fn();
    render(<CoverageVerdict view={view} onDrillDown={fn} />);
    screen.getByText("定价").click();
    expect(fn).toHaveBeenCalledWith("pricing");
  });
});
```

- [ ] **Step 2: 运行确认失败**

Run: `npm run test -- src/__tests__/features/CoverageVerdict.test.tsx`
Expected: FAIL(模块不存在)

- [ ] **Step 3: 实现**

`CoverageVerdict.tsx`——props `{ view: CompletenessView; onDrillDown: (dimKey: string) => void }`。遍历 `view.dimensionList`,每维一行:维度中文名 + StatusBadge(state→tone:verified=running/draft=held/missing=blocked/methodology=brand)+ 大白话后果文案 + 行可点击触发 onDrillDown。文案映射(照 mockup `cockpit-final.html`):verified→「有已验证的硬事实,AI 可放心讲」、draft→「有 N 条草稿没核验,审过前 AI 不用」、missing(尤其 effectClaims)→「一条都没有,AI 讲了会被安全闸当场拦下」、methodology→「只有方法论,能讲思路不能给硬承诺」。复用现有 StatusBadge 组件(`components/ui/StatusBadge`)。CSS 无竖色杠,纯白卡 hover 上浮。

- [ ] **Step 4: 运行确认通过**

Run: `npm run test -- src/__tests__/features/CoverageVerdict.test.tsx`
Expected: PASS(3 用例)

- [ ] **Step 5: Commit**

```bash
git add frontend/src/features/knowledge/cockpit/CoverageVerdict.tsx frontend/src/features/knowledge/cockpit/CoverageVerdict.module.css frontend/src/__tests__/features/CoverageVerdict.test.tsx
git commit -m "feat(kb-frontend): 5 维大白话裁决组件"
```

### Task 6: CockpitView 主屏 + 接进 StewardMode 默认 pane

**Files:**
- Create: `frontend/src/features/knowledge/cockpit/CockpitView.tsx` + `.module.css`
- Modify: `frontend/src/features/knowledge/index.tsx`(StewardMode:加 cockpit pane + 改默认 + nav 按钮)
- Test: `frontend/src/__tests__/features/CockpitView.test.tsx`

- [ ] **Step 1: 失败测试**

```tsx
import { render, screen, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { CockpitView } from "../../features/knowledge/cockpit/CockpitView";

beforeEach(() => {
  global.fetch = vi.fn((url: string) => {
    if (String(url).includes("/completeness")) {
      return Promise.resolve({ ok: true, json: () => Promise.resolve({
        answeringMode: "product_safe", needsReviewChunks: 12,
        coverage: { effectClaims: { state: "missing" } },
      }) } as Response);
    }
    if (String(url).includes("/integrity-report")) {
      return Promise.resolve({ ok: true, json: () => Promise.resolve({ item: { total: 40, needsReview: 12, rejected: 3 } }) } as Response);
    }
    return Promise.resolve({ ok: true, json: () => Promise.resolve({}) } as Response);
  }) as unknown as typeof fetch;
});

describe("CockpitView", () => {
  it("加载后显示 answeringMode 仪表 + 5 维裁决 + 待办计数", async () => {
    render(<CockpitView onOpenReview={() => {}} onOpenAutoVerify={() => {}} />);
    await waitFor(() => expect(screen.getByText(/可安全讲产品/)).toBeInTheDocument());
    expect(screen.getByText("效果数据")).toBeInTheDocument();
    expect(screen.getByText("12")).toBeInTheDocument(); // 待审草稿计数
  });
});
```

- [ ] **Step 2: 运行确认失败**

Run: `npm run test -- src/__tests__/features/CockpitView.test.tsx`
Expected: FAIL(模块不存在)

- [ ] **Step 3: 实现 CockpitView**

`CockpitView.tsx`——props `{ onOpenReview: (dimKey?: string) => void; onOpenAutoVerify: () => void }`。useEffect 并行 fetch `/api/operation-knowledge/completeness` + `/api/operation-knowledge/integrity-report`,用 parseCompleteness/parseIntegrityReport 解析。渲染:`<AnsweringModeGauge>` + `<CoverageVerdict onDrillDown={onOpenReview}>` + 待办区(3 个 MetricCard:needsReviewChunks/降级数/gap 数 + auto-verify 入口按钮 onOpenAutoVerify)。降级数暂用 integrity 的 rejected 占位(真实 distortion 计数阶段三补)。MetricCard 复用 `components/ui/MetricCard`。

- [ ] **Step 4: 接进 StewardMode**

index.tsx StewardMode(266):
- `useState` pane 类型加 `"cockpit"`,默认值 `"lint"` → `"cockpit"`(行 267)
- nav 顶部加 cockpit 按钮(`<Activity>` 或 `<ShieldCheck>` 图标,标签「治理总览」)
- 主区加 `{pane === "cockpit" && <CockpitView onOpenReview={() => setPane("review")} onOpenAutoVerify={() => setPane("autoVerify")} />}`(autoVerify pane 阶段四加,先用 review 占位避免类型错:暂 `onOpenAutoVerify={() => setPane("review")}`)
- import CockpitView

- [ ] **Step 5: 运行测试 + 构建**

Run: `npm run test -- src/__tests__/features/CockpitView.test.tsx && npm run build`
Expected: 测试 PASS;tsc 0 error

- [ ] **Step 6: Commit**

```bash
git add frontend/src/features/knowledge/cockpit/ frontend/src/__tests__/features/CockpitView.test.tsx frontend/src/features/knowledge/index.tsx
git commit -m "feat(kb-frontend): 治理驾驶舱主屏 CockpitView + 接进 Steward 默认 pane"
```

---

## 阶段三:审核 + 对话双栏(review pane 升级)

把 mockup `review-chat-v2.html` + `single-action.html` + `plain-language.html` 落地。**核心红线:单一「让 AI 可以用这条」键 = 前端顺序调 chat_apply→verify,任一失败回滚;未过 D2 闸时禁用。**

### Task 7: 放行检查纯函数(D2 闸的前端镜像)

把后端 D2 闸逻辑(verified 必须 source_quote + source_anchors 双非空)做成前端纯函数,驱动生效键禁用态。**这是红线在前端的镜像,不是替代后端校验。**

**Files:**
- Modify: `frontend/src/features/knowledge/trustTypes.ts`(加 canGoLive)
- Test: `frontend/src/__tests__/features/trustTypes.test.ts`(append)

- [ ] **Step 1: append 失败测试**

```ts
import { canGoLive } from "../../features/knowledge/trustTypes";

describe("canGoLive(D2 闸前端镜像)", () => {
  it("有原话+有锚点 → 可生效", () => {
    expect(canGoLive({ hasQuote: true, hasAnchor: true }).ok).toBe(true);
  });
  it("缺锚点 → 不可生效,理由含「来源/出处」", () => {
    const r = canGoLive({ hasQuote: true, hasAnchor: false });
    expect(r.ok).toBe(false);
    expect(r.missing).toContain("anchor");
  });
  it("缺原话 → 不可生效", () => {
    expect(canGoLive({ hasQuote: false, hasAnchor: true }).ok).toBe(false);
  });
});
```

- [ ] **Step 2: 运行确认失败**

Run: `npm run test -- src/__tests__/features/trustTypes.test.ts`
Expected: FAIL("canGoLive is not a function")

- [ ] **Step 3: 实现**

trustTypes.ts 追加:

```ts
export interface GoLiveCheck { ok: boolean; missing: ("quote" | "anchor")[]; }

export function canGoLive(input: { hasQuote: boolean; hasAnchor: boolean }): GoLiveCheck {
  const missing: ("quote" | "anchor")[] = [];
  if (!input.hasQuote) missing.push("quote");
  if (!input.hasAnchor) missing.push("anchor");
  return { ok: missing.length === 0, missing };
}
```

- [ ] **Step 4: 运行确认通过 + commit**

Run: `npm run test -- src/__tests__/features/trustTypes.test.ts`
Expected: PASS

```bash
git add frontend/src/features/knowledge/trustTypes.ts frontend/src/__tests__/features/trustTypes.test.ts
git commit -m "feat(kb-frontend): canGoLive 放行检查纯函数(D2 闸前端镜像)"
```

### Task 8: 单一「让 AI 可以用这条」动作 hook(apply→verify 串调)

**Files:**
- Create: `frontend/src/features/knowledge/cockpit/useGoLive.ts`
- Test: `frontend/src/__tests__/features/useGoLive.test.ts`

- [ ] **Step 1: 失败测试**

```ts
import { describe, it, expect, vi, beforeEach } from "vitest";
import { runGoLive } from "../../features/knowledge/cockpit/useGoLive";

describe("runGoLive(apply→verify 串调)", () => {
  it("apply 成功后才调 verify,两步都成功返回 ok", async () => {
    const calls: string[] = [];
    global.fetch = vi.fn((url: string) => {
      calls.push(String(url));
      return Promise.resolve({ ok: true, json: () => Promise.resolve({}) } as Response);
    }) as unknown as typeof fetch;
    const r = await runGoLive({ sessionId: "s1", chunkId: "c1" });
    expect(r.ok).toBe(true);
    expect(calls[0]).toContain("/chat/s1/apply");
    expect(calls[1]).toContain("/chunks/c1/verify");
  });
  it("verify 被 D2 闸拒(4xx)→ 返回 needsAnchor,不抛错", async () => {
    global.fetch = vi.fn((url: string) =>
      Promise.resolve(String(url).includes("/verify")
        ? ({ ok: false, status: 400, json: () => Promise.resolve({ error: "缺 source_anchors" }) } as Response)
        : ({ ok: true, json: () => Promise.resolve({}) } as Response))
    ) as unknown as typeof fetch;
    const r = await runGoLive({ sessionId: "s1", chunkId: "c1" });
    expect(r.ok).toBe(false);
    expect(r.reason).toBe("gate_blocked");
  });
});
```

- [ ] **Step 2: 运行确认失败**

Run: `npm run test -- src/__tests__/features/useGoLive.test.ts`
Expected: FAIL(模块不存在)

- [ ] **Step 3: 实现**

`useGoLive.ts` 导出 `runGoLive({ sessionId?, chunkId })`:若有 sessionId(对话改过)先 POST `/api/operation-knowledge/chat/${sessionId}/apply`,失败返回 `{ok:false, reason:"apply_failed"}`;成功后 POST `/api/operation-knowledge/chunks/${chunkId}/verify`,4xx → `{ok:false, reason:"gate_blocked"}`,5xx → `{ok:false, reason:"server_error"}`,成功 → `{ok:true}`。无 sessionId(直接审核未对话)则跳过 apply 直接 verify。同时导出一个 `useGoLive` React hook 包装(管 pending/error 态)。

- [ ] **Step 4: 运行确认通过 + commit**

Run: `npm run test -- src/__tests__/features/useGoLive.test.ts`
Expected: PASS(2 用例)

```bash
git add frontend/src/features/knowledge/cockpit/useGoLive.ts frontend/src/__tests__/features/useGoLive.test.ts
git commit -m "feat(kb-frontend): 单一生效动作 apply→verify 串调 hook"
```

### Task 9: ReviewChat 双栏组件 + 替换 ReviewView

**Files:**
- Create: `frontend/src/features/knowledge/cockpit/ReviewChat.tsx` + `.module.css`
- Modify: `frontend/src/features/knowledge/index.tsx`(StewardMode review pane 用新组件)
- Test: `frontend/src/__tests__/features/ReviewChat.test.tsx`

- [ ] **Step 1: 失败测试**

```tsx
import { render, screen } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { ReviewChat } from "../../features/knowledge/cockpit/ReviewChat";

const chunk = {
  id: "c1", title: "企业版年费 12800", summary: "含 5 个坐席",
  sourceQuote: "企业版一年 12800", sourceAnchors: [{ startLine: 1 }],
  integrityStatus: "needs_review", status: "draft",
};

describe("ReviewChat", () => {
  it("左栏裁决:双检查全过显示「可以生效」+ 生效键可用", () => {
    render(<ReviewChat chunk={chunk as never} onResolved={() => {}} />);
    expect(screen.getByText(/可以生效|让 AI 可以用/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /让 AI 可以用这条/ })).toBeEnabled();
  });
  it("缺锚点 → 生效键禁用 + 大白话说明", () => {
    render(<ReviewChat chunk={{ ...chunk, sourceAnchors: [] } as never} onResolved={() => {}} />);
    expect(screen.getByRole("button", { name: /让 AI 可以用这条/ })).toBeDisabled();
    expect(screen.getByText(/这话.*哪来|来源|出处/)).toBeInTheDocument();
  });
  it("右栏对话标题写明「只动这条 · 改完仍由你放行」", () => {
    render(<ReviewChat chunk={chunk as never} onResolved={() => {}} />);
    expect(screen.getByText(/只动这条/)).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 运行确认失败**

Run: `npm run test -- src/__tests__/features/ReviewChat.test.tsx`
Expected: FAIL(模块不存在)

- [ ] **Step 3: 实现 ReviewChat**

`ReviewChat.tsx`——props `{ chunk: ReviewChunkItem; onResolved: () => void }`。左栏:一句话裁决(canGoLive 决定绿环「可以生效」/琥珀环「还差一样」)+ 内容(大白话「这条说的是」)+ 原话出处引用块 + 承诺背书提示(chunkType==="product_fact" 或含报价时显示🛡️)+ 单一生效键(canGoLive.ok 决定 enabled,onClick 调 runGoLive,成功后 onResolved)。右栏:对话工坊(标题「问 AI 改这条 · 只动这条 · 改完仍由你放行」,POST `/api/operation-knowledge/chat` 带 attachments:[{chunk_id}],展示 turns,AI 改完左栏高亮预览 + 出现 apply 预览条)。文案/布局照 mockup `review-chat-v2.html` + `plain-language.html`(大白话)。

- [ ] **Step 4: 替换 ReviewView 引用**

index.tsx StewardMode:`{pane === "review" && <ReviewView />}` 改为渲染一个列表→点击进 ReviewChat 的容器(或 ReviewView 内每条「展开」改为打开 ReviewChat)。保留 ReviewView 的列表/分类逻辑,详情改用 ReviewChat。最小改动:ReviewView 的 focusChunk 由打开 Inspector 改为打开 ReviewChat 抽屉。

- [ ] **Step 5: 运行测试 + 构建**

Run: `npm run test -- src/__tests__/features/ReviewChat.test.tsx && npm run build`
Expected: 测试 3 用例 PASS;tsc 0 error

- [ ] **Step 6: Commit**

```bash
git add frontend/src/features/knowledge/cockpit/ReviewChat.tsx frontend/src/features/knowledge/cockpit/ReviewChat.module.css frontend/src/__tests__/features/ReviewChat.test.tsx frontend/src/features/knowledge/index.tsx
git commit -m "feat(kb-frontend): 审核+对话双栏 ReviewChat + 单一生效键"
```

---

## 阶段四:auto-verify 批量屏(autoVerify pane)

落地 mockup `auto-verify.html`。

### Task 10: AutoVerifyPanel 组件 + 接进 StewardMode

后端 `POST /api/operation-knowledge/auto-verify`,请求体 `{ account_id?, confidence_threshold?(默认7,0-10), human_audit_sample_rate?(默认0.1), limit?(默认50,1-500) }`,响应 `{ processed, verified, needsReview, rejected, needsHumanAudit, degraded }`(knowledge.rs:660/899-973)。

**Files:**
- Create: `frontend/src/features/knowledge/cockpit/AutoVerifyPanel.tsx` + `.module.css`
- Modify: `frontend/src/features/knowledge/index.tsx`(StewardMode 加 autoVerify pane + CockpitView onOpenAutoVerify 改真实跳转)
- Test: `frontend/src/__tests__/features/AutoVerifyPanel.test.tsx`

- [ ] **Step 1: 失败测试**

```tsx
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { AutoVerifyPanel } from "../../features/knowledge/cockpit/AutoVerifyPanel";

describe("AutoVerifyPanel", () => {
  it("默认显示三档松紧(适中选中)+ 留复查开关 + 数量", () => {
    render(<AutoVerifyPanel />);
    expect(screen.getByText("适中")).toBeInTheDocument();
    expect(screen.getByText(/留.*复查/)).toBeInTheDocument();
  });
  it("点开始筛 → 调 auto-verify,结果分三堆显示", async () => {
    global.fetch = vi.fn(() => Promise.resolve({ ok: true, json: () => Promise.resolve({
      processed: 50, verified: 31, needsReview: 14, rejected: 0, needsHumanAudit: 5,
    }) } as Response)) as unknown as typeof fetch;
    render(<AutoVerifyPanel />);
    fireEvent.click(screen.getByRole("button", { name: /开始筛/ }));
    await waitFor(() => expect(screen.getByText("31")).toBeInTheDocument());
    expect(screen.getByText("5")).toBeInTheDocument();   // 留复查
    expect(screen.getByText("14")).toBeInTheDocument();  // 没把握
  });
});
```

- [ ] **Step 2: 运行确认失败**

Run: `npm run test -- src/__tests__/features/AutoVerifyPanel.test.tsx`
Expected: FAIL(模块不存在)

- [ ] **Step 3: 实现**

`AutoVerifyPanel.tsx`——状态:松紧(宽松=5/适中=7/严格=9 映射 confidence_threshold)、留复查开关(on→sample_rate 0.1, off→0)、数量(50/100/全部→limit 50/100/500)、result。点「开始筛」POST auto-verify,结果分三堆渲染(verified=「AI 觉得没问题」绿、needsHumanAudit=「留给你复查」琥珀、needsReview+rejected=「AI 没把握没动」灰)。文案/布局照 mockup `auto-verify.html`,大白话 + 点题条「AI 不会替你放行没把握的」。

- [ ] **Step 4: 接进 StewardMode**

index.tsx:pane 类型加 `"autoVerify"`,nav 加按钮(`<Sparkles>` 图标「批量校验」),主区加 `{pane === "autoVerify" && <AutoVerifyPanel />}`。CockpitView 的 onOpenAutoVerify 改 `() => setPane("autoVerify")`(Task 6 的占位换成真实)。

- [ ] **Step 5: 运行测试 + 构建**

Run: `npm run test -- src/__tests__/features/AutoVerifyPanel.test.tsx && npm run build`
Expected: 测试 2 用例 PASS;tsc 0 error

- [ ] **Step 6: Commit**

```bash
git add frontend/src/features/knowledge/cockpit/AutoVerifyPanel.tsx frontend/src/features/knowledge/cockpit/AutoVerifyPanel.module.css frontend/src/__tests__/features/AutoVerifyPanel.test.tsx frontend/src/features/knowledge/index.tsx
git commit -m "feat(kb-frontend): auto-verify 批量校验屏 + 接进 Steward"
```

---

## 阶段五:富字段显形 + 收尾

### Task 11: chunk 富字段显形进 ReviewChat 折叠区

把 Task 3 加的富字段(usageStats/validFrom-To/lockedFields/distortionRisks)在 ReviewChat 左栏折叠区用大白话显形。

**Files:**
- Modify: `frontend/src/features/knowledge/cockpit/ReviewChat.tsx`
- Test: `frontend/src/__tests__/features/ReviewChat.test.tsx`(append)

- [ ] **Step 1: append 失败测试**

```tsx
it("显形富字段:用了多少次 / 降级痕迹 / 字段锁(大白话)", () => {
  const rich = { ...chunk,
    usageStats: { hitCount30d: 8, blockedCount30d: 2 },
    distortionRisks: ["提交为 verified 但缺锚点,已降级"],
    lockedFields: ["sourceQuote"],
  };
  render(<ReviewChat chunk={rich as never} onResolved={() => {}} />);
  expect(screen.getByText(/用了 8 次|被用过 8/)).toBeInTheDocument();
  expect(screen.getByText(/降级|为什么被打回/)).toBeInTheDocument();
});
```

- [ ] **Step 2: 运行确认失败 → 实现 → 通过**

ReviewChat 折叠区加:usageStats→「最近 30 天被 AI 用了 N 次,被拦 M 次」、distortionRisks→「为什么被打回:…」列表、lockedFields→对应字段标🔒「这项被锁定,改不了」、validFrom/To→「有效期」。全大白话。

Run: `npm run test -- src/__tests__/features/ReviewChat.test.tsx && npm run build`
Expected: PASS;tsc 0 error

- [ ] **Step 3: Commit**

```bash
git add frontend/src/features/knowledge/cockpit/ReviewChat.tsx frontend/src/__tests__/features/ReviewChat.test.tsx
git commit -m "feat(kb-frontend): chunk 富字段大白话显形(用量/降级痕迹/字段锁)"
```

### Task 12: 导入向导草稿告知 + 全量回归

**Files:**
- Modify: `frontend/src/features/knowledge/index.tsx`(ImportWizard 完成态文案 + 跳驾驶舱按钮)

- [ ] **Step 1: ImportWizard 完成态加明确告知**

ImportWizard(index.tsx:1126)导入完成后的提示强化为大白话:「导入的 N 条都是草稿,AI 还不能用。需要你逐条确认后才会生效。」+ 一个「去治理总览逐条处理 →」按钮(`window.dispatchEvent` 或 setPane 跳 cockpit)。

- [ ] **Step 2: 全量构建 + 测试**

Run: `npm run build && npm run test`
Expected: tsc 0 error;所有 __tests__ 测试 PASS

- [ ] **Step 3: 后端无人接管 lint 自查**

Run(在 worktree 根 `E:/yw/wa-kb-cockpit`):`scripts/check-no-human-takeover.sh`(或 .ps1)
Expected: 0 violations(新增前端文案不得含「人工接管/takeover/人工」等禁词——用「让 AI 可以用/逐条确认/放行」等 AI 自治措辞)

- [ ] **Step 4: Commit**

```bash
git add frontend/src/features/knowledge/index.tsx
git commit -m "feat(kb-frontend): 导入向导明确告知「全是草稿需逐条放行」+ 跳驾驶舱"
```

### Task 13: 浏览器真人走查(CLAUDE.md 要求)

- [ ] **Step 1: 起前后端**(后端用独立 target 避并发 cron 抢锁:`CARGO_TARGET_DIR=target-verify cargo run`;前端 `cd frontend && npm run dev`)
- [ ] **Step 2: 逐屏走查** cockpit 默认屏 → 点维度下钻 review → 对话改 chunk → 生效键(全过/禁用两态)→ auto-verify 批量。确认大白话文案、无竖色杠、呼吸克制、button 无全局污染。
- [ ] **Step 3: 记录走查结果**,有问题修复后再 commit。

---

## Self-Review 检查(写计划后自查)

- **Spec 覆盖**:答辩 mode 仪表(T4)、5 维矩阵(T5)、审核+对话(T9)、单一生效键(T7/T8)、auto-verify(T10)、富字段(T11)、导入告知(T12)、schema 修复(T1-T3)、信息架构升级 Steward(T6)——spec 各节均有对应任务。✓
- **占位扫描**:无 TBD;每个代码步给了完整代码或精确文件:行号 + 改法。✓
- **类型一致**:CompletenessView/CoverageFlags/TrustChunkFields/GoLiveCheck 在 T1-T3/T7 定义,后续 T4-T11 引用一致。✓
- **红线**:T7(D2 闸镜像)、T8(apply→verify 不破对话不能 verify)、T10(auto-verify 主体是运营)、T12(import 草稿告知 + no-human-takeover lint)守住 spec 红线清单。✓

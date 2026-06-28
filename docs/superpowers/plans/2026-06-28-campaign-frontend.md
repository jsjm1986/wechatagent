# 活动推送结果看板（前端）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新建一级频道 `campaign`，消费 `GET /api/campaigns/:id/sends` 把推送结果渲染成 7 桶汇总 + 每人明细表，并从总控 AI 的 `dispatch_campaign` 工具调用结果跳转进入（带 campaignId）。

**Architecture:** 单文件 feature 模块（`features/campaign/index.tsx`）+ Zustand store（`stores/campaignStore.ts`，含跨频道 `openReport` 跳转）+ CSS Modules 样式。零后端改动；纯函数 `bucketTone`/`bucketLabel` 把 7 桶映射到 StatusBadge 既有 5 tone。command-center 在 `dispatch_campaign` 成功的工具调用旁渲染一个「查看推送结果」入口。

**Tech Stack:** React 19 + TypeScript + Zustand + Vite + Vitest（jsdom + @testing-library/react）。

## Global Constraints

- **设计系统严守**：色值只用 `frontend/src/components/ui/tokens.css` 既有 CSS 变量，禁硬编码色值。蓝（`--color-scheduled` #0A84FF）仅主操作/可点击；紫（`--color-brand` #5E5CE6）仅 AI 身份，不当普通状态色。7 桶状态色全部走 StatusBadge 既有 tone class。
- **CSS 落地用 CSS Modules**：`Campaign.module.css` + `import styles from "./Campaign.module.css"`，**绝不用裸 `import "./x.css"`**（避免 Rollup tree-shake 删除副作用导入的历史坑）。
- **桶全集闭合**：`sent / pending / blocked / canceled / escalated / skipped / unknown` 七桶，贯穿 `bucketTone`/`bucketLabel`/看板渲染/测试。
- **7→5 tone 映射（固定）**：sent→running、pending→scheduled、blocked→blocked、escalated→held、canceled→inactive、skipped→inactive、unknown→inactive。
- **命名红线（CI 硬门 check-no-human-takeover 扫 frontend/src 新增行）**：禁 `人工|接管|takeover|hand-off|人工介入|人工托管`。`escalated` 标「已请示」非「转人工」；注释也不得引入禁词。
- **零后端改动**：只消费现有 4 端点，不碰任何 Rust 代码。
- **跳转守卫（防死链）**：command-center 仅当 `call.toolName === "wechatagent.dispatch_campaign"` 且 `call.status ∈ {"succeeded","executed_unverified"}` 且 `typeof call.response?.campaignId === "string"` 时才渲跳转入口。
- **基线**：分支 `feat/campaign-frontend` ← `origin/main` 700c57d。前端验证 = `cd frontend && npm run build` 编译过 + `npx vitest run src/__tests__/features/campaign` 全绿。

---

## File Structure

- **Create `frontend/src/stores/campaignStore.ts`**：Zustand store。持 `selectedCampaignId`/`report`/`loading`；action `openReport(id)`（set id + 切频道 + loadReport）/`loadReport(id)`/`clear()`。就近 export 类型 `CampaignReport`/`CampaignSummary`/`CampaignSendItem`。
- **Create `frontend/src/features/campaign/index.tsx`**：default export `CampaignFeature`。导出纯函数 `bucketTone`/`bucketLabel`（供测试）。三态渲染 + 7 桶汇总 + 明细表 + 桶筛选。
- **Create `frontend/src/features/campaign/Campaign.module.css`**：镜像 `SendAnalytics.module.css` 的 page→panel→metrics→table 结构 + 桶 chip 筛选样式。
- **Modify `frontend/src/types/index.ts`**：`Channel` 联合加 `"campaign"`。
- **Modify `frontend/src/app/channels.ts`**：加 lazy import + 一条 `ChannelDef` entry（group `"运营"`）。
- **Modify `frontend/src/features/command-center/index.tsx`**：在工具调用渲染块后加「查看推送结果」跳转入口。
- **Create `frontend/src/__tests__/features/campaign/campaign.test.tsx`**：纯函数 + 看板渲染 + 三态 + 桶筛选测试。
- **Create `frontend/src/__tests__/features/campaign/commandJump.test.tsx`**：command-center 跳转守卫测试。

任务划分：Task 1 = store（数据层，可独立测）；Task 2 = 看板组件 + CSS + 纯函数 + 渲染测试（依赖 Task 1 类型）；Task 3 = 频道接线（types + channels）；Task 4 = command-center 跳转入口 + 守卫测试（依赖 Task 1 的 openReport）。

---

### Task 1: campaignStore（数据层 + 类型）

**Files:**
- Create: `frontend/src/stores/campaignStore.ts`

**Interfaces:**
- Produces:
  - `interface CampaignReport { campaignId: string; title: string; status: string; summary: CampaignSummary; items: CampaignSendItem[] }`
  - `interface CampaignSummary { targetCount: number; sent: number; pending: number; skipped: number; unknown: number; blocked: Record<string,number>; canceled: Record<string,number>; escalated: Record<string,number> }`
  - `interface CampaignSendItem { contactWxid: string; name: string; status: string; reason?: string }`
  - `useCampaignStore` with state `{ selectedCampaignId: string|null; report: CampaignReport|null; loading: boolean }` and actions `openReport(id: string): void` / `loadReport(id: string): Promise<void>` / `clear(): void`.
- Consumes: `api.get<T>(url)`（`frontend/src/lib/api.ts`）；`useUiStore.getState().setError`（`frontend/src/stores/uiStore.ts`）；`useNavigationStore.getState().setChannel`（`frontend/src/stores/navigationStore.ts`，签名 `setChannel(channel: Channel): void`）。

- [ ] **Step 1: 写 store（含类型）**

创建 `frontend/src/stores/campaignStore.ts`：

```ts
import { create } from "zustand";
import { api } from "../lib/api";
import { useUiStore } from "./uiStore";
import { useNavigationStore } from "./navigationStore";

export interface CampaignSummary {
  targetCount: number;
  sent: number;
  pending: number;
  skipped: number;
  unknown: number;
  blocked: Record<string, number>;
  canceled: Record<string, number>;
  escalated: Record<string, number>;
}

export interface CampaignSendItem {
  contactWxid: string;
  name: string;
  status: string;
  reason?: string;
}

export interface CampaignReport {
  campaignId: string;
  title: string;
  status: string;
  summary: CampaignSummary;
  items: CampaignSendItem[];
}

interface CampaignState {
  selectedCampaignId: string | null;
  report: CampaignReport | null;
  loading: boolean;
  openReport: (id: string) => void;
  loadReport: (id: string) => Promise<void>;
  clear: () => void;
}

export const useCampaignStore = create<CampaignState>((set, get) => ({
  selectedCampaignId: null,
  report: null,
  loading: false,
  openReport: (id) => {
    set({ selectedCampaignId: id, report: null });
    useNavigationStore.getState().setChannel("campaign");
    void get().loadReport(id);
  },
  loadReport: async (id) => {
    set({ loading: true });
    try {
      const r = await api.get<CampaignReport>(`/api/campaigns/${id}/sends`);
      set({ report: r });
    } catch (e) {
      useUiStore.getState().setError(e instanceof Error ? e.message : String(e));
    } finally {
      set({ loading: false });
    }
  },
  clear: () => set({ selectedCampaignId: null, report: null, loading: false }),
}));
```

> 说明：`openReport` 先 `set` 选中 id 并清旧 report，再切频道，再异步 loadReport——切频道与加载并行，频道挂载时看到 `loading=true`。`setChannel("campaign")` 依赖 Task 3 已把 `"campaign"` 加入 `Channel` 联合；若 Task 3 未先做，TS 会报 `"campaign"` 不在 Channel 联合——这是预期的跨任务依赖，实现者按 Task 顺序执行即可（Task 3 在 Task 4 用到跳转前完成）。本 Task 单测不触发 setChannel 的类型检查路径（mock 掉 navigationStore）。

- [ ] **Step 2: 写 store 单测**

创建 `frontend/src/__tests__/features/campaign/store.test.ts`：

```ts
import { describe, it, expect, vi, beforeEach } from "vitest";
import { useCampaignStore } from "../../../stores/campaignStore";
import type { CampaignReport } from "../../../stores/campaignStore";
import { api } from "../../../lib/api";

vi.mock("../../../lib/api", () => ({
  api: { get: vi.fn() },
}));
vi.mock("../../../stores/navigationStore", () => ({
  useNavigationStore: { getState: () => ({ setChannel: vi.fn() }) },
}));
vi.mock("../../../stores/uiStore", () => ({
  useUiStore: { getState: () => ({ setError: vi.fn() }) },
}));

const sample: CampaignReport = {
  campaignId: "c1",
  title: "双11老客续费7折",
  status: "completed",
  summary: {
    targetCount: 3, sent: 1, pending: 1, skipped: 0, unknown: 0,
    blocked: { daily_limit: 1 }, canceled: {}, escalated: {},
  },
  items: [
    { contactWxid: "a", name: "张三", status: "sent" },
    { contactWxid: "b", name: "李四", status: "pending" },
    { contactWxid: "c", name: "王五", status: "blocked", reason: "daily_limit" },
  ],
};

describe("campaignStore", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useCampaignStore.setState({ selectedCampaignId: null, report: null, loading: false });
  });

  it("loadReport 成功写入 report 并清 loading", async () => {
    (api.get as any).mockResolvedValue(sample);
    await useCampaignStore.getState().loadReport("c1");
    const s = useCampaignStore.getState();
    expect(s.report).toEqual(sample);
    expect(s.loading).toBe(false);
    expect(api.get).toHaveBeenCalledWith("/api/campaigns/c1/sends");
  });

  it("loadReport 失败时不抛、loading 归位、report 保持 null", async () => {
    (api.get as any).mockRejectedValue(new Error("boom"));
    await useCampaignStore.getState().loadReport("c1");
    const s = useCampaignStore.getState();
    expect(s.report).toBeNull();
    expect(s.loading).toBe(false);
  });

  it("openReport 设置 selectedCampaignId 并触发加载", async () => {
    (api.get as any).mockResolvedValue(sample);
    useCampaignStore.getState().openReport("c1");
    expect(useCampaignStore.getState().selectedCampaignId).toBe("c1");
    // openReport 内部 void loadReport——等微任务跑完
    await Promise.resolve();
    await Promise.resolve();
    expect(api.get).toHaveBeenCalledWith("/api/campaigns/c1/sends");
  });

  it("clear 重置全部", () => {
    useCampaignStore.setState({ selectedCampaignId: "x", report: sample, loading: true });
    useCampaignStore.getState().clear();
    const s = useCampaignStore.getState();
    expect(s.selectedCampaignId).toBeNull();
    expect(s.report).toBeNull();
    expect(s.loading).toBe(false);
  });
});
```

- [ ] **Step 3: 跑测试确认通过 + 编译**

```
cd frontend && npx vitest run src/__tests__/features/campaign/store.test.ts 2>&1 | tail -15
cd frontend && npx tsc --noEmit 2>&1 | grep -E "campaignStore|error" | head -10
```
Expected: 4 测试 PASS。`tsc --noEmit` 对 campaignStore 无 error（注：`setChannel("campaign")` 在 Task 3 前会报 Channel 联合不含 "campaign"——若本步在 Task 3 之前执行，此 TS error 是预期的、Task 3 修复；store 测试因 mock 了 navigationStore 不受影响仍 PASS）。

- [ ] **Step 4: 提交**

```bash
git add frontend/src/stores/campaignStore.ts frontend/src/__tests__/features/campaign/store.test.ts
git commit -m "feat(campaign-fe): campaignStore 推送结果数据层(openReport跨频道跳转+loadReport)+单测"
```

---

### Task 2: 看板组件 + 纯函数 + 样式

**Files:**
- Create: `frontend/src/features/campaign/index.tsx`
- Create: `frontend/src/features/campaign/Campaign.module.css`
- Create: `frontend/src/__tests__/features/campaign/campaign.test.tsx`

**Interfaces:**
- Consumes: `useCampaignStore`（Task 1）+ 类型 `CampaignReport`/`CampaignSendItem`；`StatusBadge`/`StatusTone`（`../../components/ui/StatusBadge`）；`EmptyState`（`../../components/ui/EmptyState`）。
- Produces:
  - default export `CampaignFeature`（供 channels.ts lazy import）。
  - `export function bucketTone(bucket: string): StatusTone`。
  - `export function bucketLabel(bucket: string): string`。

- [ ] **Step 1: 写纯函数测试 + 渲染测试**

创建 `frontend/src/__tests__/features/campaign/campaign.test.tsx`：

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import CampaignFeature, { bucketTone, bucketLabel } from "../../../features/campaign";
import { useCampaignStore } from "../../../stores/campaignStore";
import type { CampaignReport } from "../../../stores/campaignStore";

vi.mock("../../../stores/campaignStore");

const report: CampaignReport = {
  campaignId: "c1",
  title: "双11老客续费7折",
  status: "completed",
  summary: {
    targetCount: 5, sent: 2, pending: 1, skipped: 0, unknown: 0,
    blocked: { daily_limit: 1 }, canceled: {}, escalated: { blocked_unverified_product_claim: 1 },
  },
  items: [
    { contactWxid: "a", name: "张三", status: "sent" },
    { contactWxid: "b", name: "李四", status: "sent" },
    { contactWxid: "c", name: "王五", status: "pending" },
    { contactWxid: "d", name: "赵六", status: "blocked", reason: "daily_limit" },
    { contactWxid: "e", name: "", status: "escalated", reason: "blocked_unverified_product_claim" },
  ],
};

describe("bucketTone / bucketLabel", () => {
  it("7 桶映射到正确 tone", () => {
    expect(bucketTone("sent")).toBe("running");
    expect(bucketTone("pending")).toBe("scheduled");
    expect(bucketTone("blocked")).toBe("blocked");
    expect(bucketTone("escalated")).toBe("held");
    expect(bucketTone("canceled")).toBe("inactive");
    expect(bucketTone("skipped")).toBe("inactive");
    expect(bucketTone("unknown")).toBe("inactive");
    expect(bucketTone("天外飞仙")).toBe("inactive"); // 兜底
  });
  it("7 桶中文标签", () => {
    expect(bucketLabel("sent")).toBe("已送达");
    expect(bucketLabel("escalated")).toBe("已请示");
    expect(bucketLabel("blocked")).toBe("被拦");
    expect(bucketLabel("unknown")).toBe("未知");
  });
});

describe("CampaignFeature", () => {
  beforeEach(() => vi.clearAllMocks());

  it("selectedCampaignId=null 渲 EmptyState", () => {
    (useCampaignStore as any).mockReturnValue({
      selectedCampaignId: null, report: null, loading: false, loadReport: vi.fn(),
    });
    render(<CampaignFeature />);
    expect(screen.getByText("暂无活动结果")).toBeInTheDocument();
  });

  it("有 report 渲汇总数值 + 明细表行数 = items 长度", () => {
    (useCampaignStore as any).mockReturnValue({
      selectedCampaignId: "c1", report, loading: false, loadReport: vi.fn(),
    });
    render(<CampaignFeature />);
    // 标题
    expect(screen.getByText("双11老客续费7折")).toBeInTheDocument();
    // sent 汇总值 2（用 testid 精确取，避免与表格数字串扰）
    expect(screen.getByTestId("metric-sent")).toHaveTextContent("2");
    expect(screen.getByTestId("metric-pending")).toHaveTextContent("1");
    // escalated reason 二级细分
    expect(screen.getByText(/blocked_unverified_product_claim/)).toBeInTheDocument();
    // 明细表行数（tbody tr）= 5
    expect(screen.getAllByTestId("detail-row")).toHaveLength(5);
    // 空 name 渲 —
    expect(screen.getByTestId("detail-row-e")).toHaveTextContent("—");
  });

  it("空 items 渲明细空态", () => {
    (useCampaignStore as any).mockReturnValue({
      selectedCampaignId: "c1",
      report: { ...report, items: [], summary: { ...report.summary, targetCount: 0, sent: 0, pending: 0, blocked: {}, escalated: {} } },
      loading: false, loadReport: vi.fn(),
    });
    render(<CampaignFeature />);
    expect(screen.getByText("暂无推送明细")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

```
cd frontend && npx vitest run src/__tests__/features/campaign/campaign.test.tsx 2>&1 | tail -15
```
Expected: FAIL（模块 `../../../features/campaign` 不存在 / 导出缺失）。

- [ ] **Step 3: 写看板组件**

创建 `frontend/src/features/campaign/index.tsx`：

```tsx
import { useEffect, useState } from "react";
import { Megaphone } from "lucide-react";
import { EmptyState } from "../../components/ui/EmptyState";
import { StatusBadge, type StatusTone } from "../../components/ui/StatusBadge";
import { useCampaignStore } from "../../stores/campaignStore";
import type { CampaignSendItem } from "../../stores/campaignStore";
import styles from "./Campaign.module.css";

const BUCKETS = ["sent", "pending", "blocked", "escalated", "canceled", "skipped", "unknown"] as const;

export function bucketTone(bucket: string): StatusTone {
  switch (bucket) {
    case "sent": return "running";
    case "pending": return "scheduled";
    case "blocked": return "blocked";
    case "escalated": return "held";
    default: return "inactive"; // canceled / skipped / unknown / 未知值
  }
}

export function bucketLabel(bucket: string): string {
  switch (bucket) {
    case "sent": return "已送达";
    case "pending": return "在途";
    case "blocked": return "被拦";
    case "escalated": return "已请示";
    case "canceled": return "已取消";
    case "skipped": return "去重跳过";
    default: return "未知";
  }
}

// 标量桶取 summary 上的计数；reason 桶取子 map 总和。
function bucketCount(summary: Record<string, unknown>, bucket: string): number {
  const v = summary[bucket];
  if (typeof v === "number") return v;
  if (v && typeof v === "object") {
    return Object.values(v as Record<string, number>).reduce((a, b) => a + b, 0);
  }
  return 0;
}

export default function CampaignFeature() {
  const { selectedCampaignId, report, loadReport } = useCampaignStore();
  const [filter, setFilter] = useState<string>("all");

  // 直接切到本频道（未经 openReport）且有选中 id 但无 report 且不在加载中 → 补一次加载。
  const loading = useCampaignStore((s) => s.loading);
  useEffect(() => {
    if (selectedCampaignId && !report && !loading) void loadReport(selectedCampaignId);
  }, [selectedCampaignId, report, loading, loadReport]);

  if (!selectedCampaignId) {
    return (
      <div className={styles.page}>
        <EmptyState
          icon={<Megaphone size={28} />}
          title="暂无活动结果"
          hint="在 AI 总控 dispatch 活动后，点「查看推送结果」进入这里查看真实触达分布。"
        />
      </div>
    );
  }

  const summary = report?.summary;
  const items: CampaignSendItem[] = report?.items ?? [];
  const shown = filter === "all" ? items : items.filter((it) => it.status === filter);

  const reasonMap = (bucket: "blocked" | "canceled" | "escalated"): Record<string, number> =>
    (summary?.[bucket] as Record<string, number> | undefined) ?? {};

  return (
    <div className={styles.page}>
      <section className={styles.panel}>
        <div className={styles.head}>
          <div className={styles.headL}>
            <span className={styles.eyebrow}>Campaign Result</span>
            <span className={styles.title}>{report ? report.title : "—"}</span>
          </div>
          {report && <StatusBadge tone="scheduled">{report.status}</StatusBadge>}
        </div>

        <div className={styles.metrics}>
          {BUCKETS.map((b) => (
            <div key={b} className={styles.metric} data-testid={`metric-${b}`}>
              <span className={styles.metricLabel}>{bucketLabel(b)}</span>
              <span className={styles.metricValue}>{summary ? bucketCount(summary as unknown as Record<string, unknown>, b) : "—"}</span>
              {(b === "blocked" || b === "canceled" || b === "escalated") && summary && (
                <div className={styles.reasons}>
                  {Object.entries(reasonMap(b)).map(([reason, n]) => (
                    <span key={reason} className={styles.reasonItem}>{reason} ×{n}</span>
                  ))}
                </div>
              )}
            </div>
          ))}
        </div>
      </section>

      <section className={styles.panel}>
        <div className={styles.head}>
          <div className={styles.headL}>
            <span className={styles.eyebrow}>Per-Contact</span>
            <span className={styles.title}>推送明细</span>
          </div>
        </div>

        <div className={styles.filters} role="tablist">
          <button
            type="button"
            className={`${styles.chip} ${filter === "all" ? styles.chipActive : ""}`}
            onClick={() => setFilter("all")}
          >
            全部 ({items.length})
          </button>
          {BUCKETS.map((b) => (
            <button
              key={b}
              type="button"
              className={`${styles.chip} ${filter === b ? styles.chipActive : ""}`}
              onClick={() => setFilter(b)}
            >
              {bucketLabel(b)}
            </button>
          ))}
        </div>

        {shown.length === 0 ? (
          <EmptyState title="暂无推送明细" hint="该筛选下没有客户记录。" />
        ) : (
          <table className={styles.table}>
            <thead>
              <tr className={styles.tr}>
                <th className={`${styles.th} ${styles.thName}`}>客户</th>
                <th className={styles.th}>wxid</th>
                <th className={styles.th}>状态</th>
                <th className={styles.th}>原因</th>
              </tr>
            </thead>
            <tbody>
              {shown.map((it) => (
                <tr key={it.contactWxid} className={styles.tr} data-testid={`detail-row-${it.contactWxid}`} {...{ "data-row": "1" }}>
                  <td className={`${styles.td} ${styles.tdName}`}>{it.name || "—"}</td>
                  <td className={`${styles.td} ${styles.tdWxid}`}>{it.contactWxid}</td>
                  <td className={styles.td}><StatusBadge tone={bucketTone(it.status)}>{bucketLabel(it.status)}</StatusBadge></td>
                  <td className={styles.td}>{it.reason || "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
    </div>
  );
}
```

> 测试用 `getAllByTestId("detail-row")`，但行上写的是 `detail-row-${wxid}`——testid 必须精确匹配。**修正**：把行 testid 拆成两个属性以同时支持「按数量取」和「按 wxid 取」。改用 `data-testid={"detail-row"}` 配合 `data-wxid`，再加一个具名 testid。最简实现：给 `<tr>` 同时挂 `data-testid="detail-row"`（计数用）— 但 testid 需唯一性时 getAllByTestId 支持重复。空 name 行单独验证用 `getByText` 范围查询。见下方 Step 3a 修正。

- [ ] **Step 3a: 修正行 testid（保证测试可取）**

把上面 `<tr>` 一行替换为（行级 testid 用固定值 `detail-row` 供 `getAllByTestId` 计数；空 name 行的 `—` 用该行内 `getByText` 验证）：

```tsx
                <tr key={it.contactWxid} className={styles.tr} data-testid="detail-row">
```

并把 campaign.test.tsx 中 `getByTestId("detail-row-e")` 改为：

```tsx
    // 空 name 行渲 —（最后一行 e 的客户列）
    const rows = screen.getAllByTestId("detail-row");
    expect(rows[4]).toHaveTextContent("—");
```

> 说明：`getAllByTestId("detail-row")` 返回全部明细行用于计数与按序索引；空 name 行（items[4]）客户列渲 `—`。这样测试与组件 testid 一致，无需每行唯一 testid。

- [ ] **Step 4: 写样式（镜像 SendAnalytics.module.css）**

创建 `frontend/src/features/campaign/Campaign.module.css`：

```css
/* 活动推送结果看板 —— 镜像 SendAnalytics 的企业白色基调：实色白卡 + tokens.css 变量。
   色值统一走 tokens.css 变量，禁止硬编码。 */

.page { position: relative; display: grid; gap: 18px; }

.panel {
  border-radius: var(--r-lg);
  padding: 22px 26px 20px;
  background: var(--surface-card);
  border: 1px solid var(--hairline);
  box-shadow: 0 14px 34px -24px rgba(20, 30, 60, .3), inset 0 1px 1px rgba(255, 255, 255, .9);
}

.head { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 16px; }
.headL { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
.eyebrow { font-size: 11px; color: var(--ink-3); font-weight: 600; letter-spacing: .2px; text-transform: uppercase; }
.title { font-size: 15px; color: var(--ink-1); font-weight: 600; letter-spacing: -.3px; }

/* 7 桶汇总 —— 扁平网格，不嵌套卡片 */
.metrics {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 14px;
}
@media (max-width: 860px) { .metrics { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
@media (max-width: 480px) { .metrics { grid-template-columns: 1fr; } }
.metric {
  display: flex; flex-direction: column; gap: 6px;
  padding: 14px 16px; border-radius: var(--r-md);
  border: 1px solid var(--hairline); background: var(--fill-inactive);
}
.metricLabel { font-size: 11.5px; color: var(--ink-3); font-weight: 600; letter-spacing: -.1px; }
.metricValue { font-size: 24px; color: var(--ink-1); font-weight: 700; letter-spacing: -.5px; line-height: 1.1; }
.reasons { display: flex; flex-wrap: wrap; gap: 4px 8px; margin-top: 4px; }
.reasonItem { font-size: 10.5px; color: var(--ink-3); letter-spacing: -.1px; font-variant-numeric: tabular-nums; }

/* 桶筛选 chip */
.filters { display: flex; flex-wrap: wrap; gap: 8px; margin-bottom: 14px; }
.chip {
  font: inherit; font-size: 12px; font-weight: 600; letter-spacing: -.1px;
  min-height: 0; padding: 7px 14px; border-radius: var(--r-sm); cursor: pointer;
  color: var(--ink-2); background: var(--surface-card);
  border: 1px solid var(--hairline);
  box-shadow: inset 0 1px 1px rgba(255, 255, 255, .9);
  transition: background .18s, border-color .18s, color .18s;
}
.chip:hover { border-color: rgba(10, 132, 255, .3); background: rgba(10, 132, 255, .03); }
.chipActive {
  color: #fff; background: var(--color-scheduled);
  border-color: rgba(10, 132, 255, .4);
  box-shadow: inset 0 1px 1px rgba(255, 255, 255, .35);
}
.chipActive:hover { background: #0a78ec; }

/* 明细表 —— 固定行高、不外包卡片 */
.table { width: 100%; border-collapse: collapse; }
.tr { border-bottom: 1px solid var(--hairline); }
.tr:last-child { border-bottom: none; }
.th {
  text-align: left; padding: 10px 12px;
  font-size: 11px; font-weight: 600; letter-spacing: .2px; text-transform: uppercase;
  color: var(--ink-3); white-space: nowrap;
}
.td { padding: 11px 12px; font-size: 13px; color: var(--ink-1); letter-spacing: -.1px; }
.tdName { font-weight: 600; }
.thName { width: 30%; }
.tdWxid { color: var(--ink-3); font-variant-numeric: tabular-nums; max-width: 220px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.table tbody .tr:hover { background: rgba(10, 132, 255, .02); }
```

> 注：若 `--fill-inactive`/`--fill-brand`/`--r-md` 等变量在 tokens.css 不存在，实现者先 grep `frontend/src/components/ui/tokens.css` 确认现有变量名，缺失则改用最接近的现有变量（SendAnalytics.module.css 已用 `--fill-inactive`/`--r-md`/`--surface-card`/`--hairline`/`--ink-1..3`/`--color-scheduled`/`--color-brand`，均存在，可直接复用）。

- [ ] **Step 5: 跑测试确认通过 + 编译**

```
cd frontend && npx vitest run src/__tests__/features/campaign/campaign.test.tsx 2>&1 | tail -20
cd frontend && npx tsc --noEmit 2>&1 | grep -E "features/campaign|error TS" | head -10
```
Expected: 全部测试 PASS；tsc 对 features/campaign 无 error（`Channel` 联合在 Task 3 加 "campaign" 前，本组件不引用 Channel 类型，故无关）。

- [ ] **Step 6: 提交**

```bash
git add frontend/src/features/campaign/index.tsx frontend/src/features/campaign/Campaign.module.css frontend/src/__tests__/features/campaign/campaign.test.tsx
git commit -m "feat(campaign-fe): 推送结果看板组件(7桶汇总+明细表+桶筛选)+bucketTone/Label纯函数+测试"
```

---

### Task 3: 频道接线（types + channels）

**Files:**
- Modify: `frontend/src/types/index.ts`（`Channel` 联合，约 :4-27）
- Modify: `frontend/src/app/channels.ts`（import 块 :24-40；CHANNELS 数组运营组）

**Interfaces:**
- Consumes: `CampaignFeature` default export（Task 2）。
- Produces: `Channel` 联合新增成员 `"campaign"`；CHANNELS 数组新增一条 `id: "campaign"` 的 ChannelDef。

- [ ] **Step 1: Channel 联合加 "campaign"**

`frontend/src/types/index.ts` 的 `Channel` 联合（末项 `| "askHumanConfig";` 之前）加一行。把：

```ts
  | "askHuman"
  | "askHumanConfig";
```

改为：

```ts
  | "askHuman"
  | "askHumanConfig"
  | "campaign";
```

- [ ] **Step 2: channels.ts 加 lazy import**

在 `frontend/src/app/channels.ts` 的 lazy import 块末尾（`const SendAnalyticsFeature = lazy(...)` 那行之后）加：

```ts
const CampaignFeature = lazy(() => import("../features/campaign"));
```

- [ ] **Step 3: channels.ts 的 lucide 图标 import 加 Megaphone**

在 `channels.ts` 顶部 `from "lucide-react"` 的 import 列表里（`BarChart3,` 之后）加一行：

```ts
  Megaphone,
```

- [ ] **Step 4: CHANNELS 数组加 campaign entry**

在 `CHANNELS` 数组里、`id: "askHumanConfig"` 那条 entry 的 `},` 之后（运营组末尾）插入：

```ts
  {
    id: "campaign",
    group: "运营",
    label: "活动",
    caption: "Campaign",
    icon: Megaphone,
    eyebrow: "Campaign",
    title: "活动推送结果",
    subtitle: "查看活动定向推送的真实触达分布：已送达 / 在途 / 被拦 / 已请示 / 已取消 / 去重跳过。从 AI 总控 dispatch 活动后点「查看推送结果」进入。",
    Component: CampaignFeature,
  },
```

- [ ] **Step 5: 编译 + 全量前端测试**

```
cd frontend && npx tsc --noEmit 2>&1 | grep -E "error TS" | head -10
cd frontend && npx vitest run src/__tests__/features/campaign 2>&1 | tail -15
```
Expected: tsc 0 error（`"campaign"` 现已在 Channel 联合，campaignStore 的 `setChannel("campaign")` 通过）；campaign 测试全绿。

- [ ] **Step 6: 提交**

```bash
git add frontend/src/types/index.ts frontend/src/app/channels.ts
git commit -m "feat(campaign-fe): 注册活动频道(Channel联合+CHANNELS entry,运营组)"
```

---

### Task 4: command-center 跳转入口 + 守卫测试

**Files:**
- Modify: `frontend/src/features/command-center/index.tsx`（工具调用渲染块，约 :282-298）
- Create: `frontend/src/__tests__/features/campaign/commandJump.test.tsx`

**Interfaces:**
- Consumes: `useCampaignStore`（Task 1，用 `getState().openReport`）；`CommandToolCall` 类型（`../../types`，字段 `toolName: string` / `status: string` / `response?: Record<string,unknown>`）。
- Produces: 一个导出的纯函数 `dispatchCampaignId(call: CommandToolCall): string | null`，便于守卫逻辑单测；command-center 渲染调用它。

- [ ] **Step 1: 写守卫纯函数测试**

创建 `frontend/src/__tests__/features/campaign/commandJump.test.tsx`：

```tsx
import { describe, it, expect } from "vitest";
import { dispatchCampaignId } from "../../../features/command-center";
import type { CommandToolCall } from "../../../types";

const call = (over: Partial<CommandToolCall>): CommandToolCall => ({
  id: "1", toolName: "wechatagent.dispatch_campaign", status: "succeeded",
  response: { campaignId: "c1" }, ...over,
});

describe("dispatchCampaignId 守卫", () => {
  it("succeeded + campaignId → 返回 id", () => {
    expect(dispatchCampaignId(call({}))).toBe("c1");
  });
  it("executed_unverified + campaignId → 返回 id", () => {
    expect(dispatchCampaignId(call({ status: "executed_unverified" }))).toBe("c1");
  });
  it("非 dispatch_campaign 工具 → null", () => {
    expect(dispatchCampaignId(call({ toolName: "wechatagent.preview_campaign" }))).toBeNull();
  });
  it("dry_run → null（防死链）", () => {
    expect(dispatchCampaignId(call({ status: "dry_run", response: {} }))).toBeNull();
  });
  it("pending_confirmation / 无 response → null", () => {
    expect(dispatchCampaignId(call({ status: "succeeded", response: undefined }))).toBeNull();
  });
  it("campaignId 非字符串 → null", () => {
    expect(dispatchCampaignId(call({ response: { campaignId: 123 } }))).toBeNull();
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

```
cd frontend && npx vitest run src/__tests__/features/campaign/commandJump.test.tsx 2>&1 | tail -10
```
Expected: FAIL（`dispatchCampaignId` 未导出）。

- [ ] **Step 3: 在 command-center 加守卫纯函数 + import**

`frontend/src/features/command-center/index.tsx` 顶部 import 区，在 `import { useCommandStore } ...` 之后加：

```ts
import { useCampaignStore } from "../../stores/campaignStore";
```

在文件内 `commandCallDetail` 函数定义之后（约 :123 后），加导出的守卫纯函数：

```ts
// 活动推送结果跳转守卫：仅当 dispatch_campaign 真实执行成功且带 campaignId 才给跳转 id，
// 否则返回 null（dry-run / 待确认 / 非该工具 / 无 id 一律不渲，防死链）。
export function dispatchCampaignId(call: CommandToolCall): string | null {
  if (call.toolName !== "wechatagent.dispatch_campaign") return null;
  if (call.status !== "succeeded" && call.status !== "executed_unverified") return null;
  const id = call.response?.campaignId;
  return typeof id === "string" ? id : null;
}
```

- [ ] **Step 4: 在工具调用渲染块加跳转入口**

把 `frontend/src/features/command-center/index.tsx` 的工具调用 `.map` 块（现为）：

```tsx
              {commandResult.toolCalls.map((call) => (
                <PlanStep
                  key={call.id || call.toolName}
                  status={planStepStatus(call)}
                  title={`${call.toolName} · ${callStatusLabel(call.status)}`}
                  detail={commandCallDetail(call)}
                />
              ))}
```

替换为（每个 call 渲 PlanStep；若是合格 dispatch_campaign 则其后追加一个跳转按钮）：

```tsx
              {commandResult.toolCalls.map((call) => {
                const campaignId = dispatchCampaignId(call);
                return (
                  <div key={call.id || call.toolName}>
                    <PlanStep
                      status={planStepStatus(call)}
                      title={`${call.toolName} · ${callStatusLabel(call.status)}`}
                      detail={commandCallDetail(call)}
                    />
                    {campaignId && (
                      <button
                        type="button"
                        className={styles.campaignJump}
                        onClick={() => useCampaignStore.getState().openReport(campaignId)}
                      >
                        查看推送结果 →
                      </button>
                    )}
                  </div>
                );
              })}
```

- [ ] **Step 5: 加跳转按钮样式**

在 `frontend/src/features/command-center/CommandCenter.module.css` 末尾追加：

```css
/* 活动推送结果跳转入口 —— 蓝色文字按钮（主操作色，可点击） */
.campaignJump {
  font: inherit; font-size: 12px; font-weight: 600; letter-spacing: -.1px;
  margin: 4px 0 8px 26px; padding: 4px 0;
  background: none; border: none; cursor: pointer;
  color: var(--color-scheduled);
}
.campaignJump:hover { text-decoration: underline; }
```

- [ ] **Step 6: 跑测试确认通过 + 全量前端测试 + 编译 + lint**

```
cd frontend && npx vitest run src/__tests__/features/campaign 2>&1 | tail -20
cd frontend && npx tsc --noEmit 2>&1 | grep -E "error TS" | head -10
cd frontend && npm run build 2>&1 | tail -8
bash scripts/check-no-human-takeover.sh 2>&1 | tail -5
```
Expected: campaign 全部测试 PASS（含 6 个守卫测试）；tsc 0 error；`npm run build` 成功；lint 0 violations。

- [ ] **Step 7: 提交**

```bash
git add frontend/src/features/command-center/index.tsx frontend/src/features/command-center/CommandCenter.module.css frontend/src/__tests__/features/campaign/commandJump.test.tsx
git commit -m "feat(campaign-fe): 总控AI dispatch_campaign 成功后加查看推送结果跳转入口+守卫测试"
```

---

### Task 5: 收口（构建 + 全量测试 + lint）

**Files:** 无新增（验证性任务）

- [ ] **Step 1: 全量前端测试 + 类型检查**

```
cd frontend && npx tsc --noEmit 2>&1 | grep -E "error TS" | head -10
cd frontend && npx vitest run 2>&1 | tail -20
```
Expected: tsc 0 error；全量 vitest 不因本功能新增失败（既有测试保持绿；新增 campaign 测试全绿）。

- [ ] **Step 2: 生产构建（SPA 真编译，CSS Modules tree-shake 验证）**

```
cd frontend && npm run build 2>&1 | tail -10
```
Expected: build 成功；产物含 campaign chunk。若 Campaign.module.css 样式丢失需排查（grep dist/assets/*.css 找 `.metric`/`.chip` 选择器），但 CSS Modules + `import styles` 已规避裸导入 tree-shake 坑。

- [ ] **Step 3: no-human-takeover lint**

```
bash scripts/check-no-human-takeover.sh 2>&1 | tail -5
```
Expected: 0 violations（看板用 已送达/被拦/已请示 等中性词）。

- [ ] **Step 4: 若本步触发任何修补则提交，否则跳过**

```bash
git add -A && git commit -m "chore(campaign-fe): 收口(构建+全量测试+lint)"
```
> 若无改动则跳过提交。

---

## 自审记录

**1. spec 覆盖**：
- §3 总体架构（新频道 + openReport 跳转）→ Task 1 store + Task 3 频道 + Task 4 跳转 ✓
- §4.1 响应类型 → Task 1 CampaignReport/Summary/SendItem ✓
- §4.2 跳转 campaignId 来源 + 显示守卫 → Task 4 dispatchCampaignId ✓
- §5.1 campaignStore → Task 1 ✓
- §5.2 三态渲染 → Task 2（null→EmptyState / loading→"—" / report→看板）+ 进频道补加载守卫 ✓
- §5.3 看板布局（page→panel→metrics→table + 桶筛选）→ Task 2 ✓
- §5.4 7→5 tone 映射 → Task 2 bucketTone ✓
- §5.5 command-center 增量 → Task 4 ✓
- §6 设计系统纪律 → Global Constraints + Task 2 CSS（tokens 变量 / CSS Modules）✓
- §7 错误处理与边界（API失败/空活动/无值/大items/命名红线）→ Task 1（catch setError）+ Task 2（空items空态、name||"—"、reason||"—"、内存筛选）✓
- §9 测试（5 类）→ Task 1 store.test + Task 2 campaign.test（纯函数/渲染/三态/筛选）+ Task 4 commandJump.test（守卫）✓

**2. 占位符扫描**：无 TBD/TODO；每个 code step 给完整代码；测试有真实断言。Step 3a 显式修正了行 testid 与测试取法的一致性。

**3. 类型一致性**：`CampaignReport`/`CampaignSummary`/`CampaignSendItem`（Task 1 定义）→ Task 2 import 一致；`bucketTone(): StatusTone`（Task 2）返回值 ∈ StatusBadge 的 StatusTone 闭集（running/scheduled/held/blocked/inactive）✓；`dispatchCampaignId(call): string|null`（Task 4）签名测试与实现一致；`Channel` 联合加 `"campaign"`（Task 3）→ campaignStore 的 `setChannel("campaign")`（Task 1）类型对齐——Task 1 在 Task 3 前会有预期 TS error，Task 3 修复，subagent 按序执行不受影响（store 单测 mock 了 navigationStore）。`useCampaignStore` selector 用法（Task 2 `useCampaignStore((s)=>s.loading)`）与对象解构混用，与 sendAnalytics/operations 既有混用范式一致。

**关键修正落实**：① PlanStep 只接 {detail,status,title} 无法内嵌按钮 → 跳转按钮作为 PlanStep 同级兄弟元素渲染（Task 4 Step 4 用 `<div>` 包裹）；② 行 testid 用固定 `detail-row` 供 getAllByTestId 计数（Step 3a）；③ tokens.css 变量名以 SendAnalytics.module.css 已用集为准（Task 2 Step 4 注）。

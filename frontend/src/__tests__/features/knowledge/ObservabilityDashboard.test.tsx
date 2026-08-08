import { render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ObservabilityDashboard } from "../../../features/knowledge/steward";

const realFetch = globalThis.fetch;

function response(ok: boolean, body: unknown): Response {
  return {
    ok,
    status: ok ? 200 : 503,
    async json() {
      return body;
    },
  } as Response;
}

describe("ObservabilityDashboard catalog envelopes", () => {
  afterEach(() => {
    globalThis.fetch = realFetch;
    vi.restoreAllMocks();
  });

  it("按真实响应包络显示已持久化目录与实时文档数", async () => {
    globalThis.fetch = vi.fn(async (url: unknown) => {
      const path = String(url);
      if (path.endsWith("/catalog/persisted")) {
        return response(true, {
          documents: [
            { catalogSummaryPersisted: "已构建", catalogFresh: true },
            { catalogSummaryPersisted: "旧快照", catalogFresh: false },
            { catalogSummaryPersisted: null, catalogFresh: false },
          ],
        });
      }
      if (path.endsWith("/operation-knowledge/catalog")) {
        return response(true, {
          item: { documents: [{ id: "d1" }, { id: "d2" }], items: [], chunks: [] },
        });
      }
      return response(false, {});
    }) as typeof fetch;

    render(<ObservabilityDashboard />);

    const card = (await screen.findByText("目录覆盖")).closest("article");
    expect(card).not.toBeNull();
    await waitFor(() => {
      const view = within(card as HTMLElement);
      expect(view.getByText("1")).toBeInTheDocument();
      expect(view.getByText("2")).toBeInTheDocument();
      expect(view.getByText("+1")).toBeInTheDocument();
    });
  });

  it("合法空目录显示 0，错误包络显示不可用而不是伪装成 0", async () => {
    let malformed = false;
    globalThis.fetch = vi.fn(async (url: unknown) => {
      const path = String(url);
      if (path.endsWith("/catalog/persisted")) {
        return response(true, malformed ? { total: 99 } : { documents: [] });
      }
      if (path.endsWith("/operation-knowledge/catalog")) {
        return response(true, malformed ? { item: { chunks: [] } } : { item: { documents: [] } });
      }
      return response(false, {});
    }) as typeof fetch;

    render(<ObservabilityDashboard />);
    const card = (await screen.findByText("目录覆盖")).closest("article") as HTMLElement;
    await waitFor(() => expect(within(card).getAllByText("0")).toHaveLength(3));

    malformed = true;
    await screen.findByRole("button", { name: "刷新" }).then((button) => button.click());
    await waitFor(() => {
      expect(within(card).getAllByText("不可用")).toHaveLength(3);
      expect(screen.getByText(/诊断数据加载失败/)).toBeInTheDocument();
    });
  });

  it("逐指标展示真实口径并将缺口比率标为历史已解决占比", async () => {
    globalThis.fetch = vi.fn(async (url: unknown) => {
      const path = String(url);
      if (path.endsWith("/admin/observability/phase-rollup")) {
        return response(true, {
          metricScopes: {
            lifecycle: { kind: "flow_window", windowHours: 24 },
            revisionReasons: { kind: "flow_window", windowHours: 24 },
            reviewerMisjudge: { kind: "flow_window", windowHours: 24 },
            negativeExamplePending: { kind: "current_inventory" },
            reviewerStats: { kind: "rolling_window_cache", windowDays: 14 },
            principalEscalations: { kind: "mixed_current_and_retained_history" },
            dealAttribution: { kind: "rolling_window_cache", windowDays: 30 },
          },
          lifecycle: [],
          revisionReasons: [],
          reviewerMisjudge: [],
          negativeExamplePending: 0,
          reviewerStats: {},
          principalEscalations: { byStatus: [] },
          dealAttribution: {},
        });
      }
      if (path.endsWith("/admin/observability/worker-health")) {
        return response(true, {
          metricScopes: {
            chatTasks: { kind: "retained_history" },
            gapSignals: { kind: "retained_history" },
            lessonsLearned: { kind: "flow_window", windowDays: 14 },
            postDecisionProjection: { kind: "mixed_current_and_retained_history" },
          },
          chatTasks: { byStatus: [] },
          gapSignals: {
            byStatus: [],
            total: 4,
            pending: 1,
            resolved: 3,
            historicalResolvedShare: 0.75,
          },
          lessonsLearned: { windowDays: 14, patternTop: [] },
          postDecisionProjection: {
            byStatus: [
              { status: "pending", count: 2 },
              { status: "failed_terminal", count: 1 },
              { status: "completed", count: 8 },
            ],
            oldestPendingAgeMs: 420000,
            attempts: { p95: 2 },
            snapshotBytes: { p95: 524288 },
            completionLatencyMs: { p95: 12500 },
            staleProfileSkips: 3,
            errorKindsTop: [{ errorKind: "llm_unavailable", count: 1 }],
          },
        });
      }
      return response(false, {});
    }) as typeof fetch;

    render(<ObservabilityDashboard />);

    expect(await screen.findByText("Phase 0-D 自治信号")).toBeInTheDocument();
    expect(screen.getAllByText("近 24 小时").length).toBeGreaterThan(0);
    expect(screen.getByText("当前库存")).toBeInTheDocument();
    expect(screen.getAllByText("保留历史").length).toBeGreaterThan(0);
    expect(screen.getByText("14 天滚动缓存")).toBeInTheDocument();
    expect(screen.getByText("30 天滚动缓存")).toBeInTheDocument();
    expect(screen.getAllByText("当前积压 + 保留历史").length).toBeGreaterThan(0);
    expect(screen.getByText("历史已解决占比")).toBeInTheDocument();
    expect(screen.getByText("75.0%")).toBeInTheDocument();
    expect(screen.getByText("发送后投影")).toBeInTheDocument();
    expect(screen.getByText("7.0 分钟")).toBeInTheDocument();
    expect(screen.getByText("12.5 秒")).toBeInTheDocument();
    expect(screen.getByText("512.0 KiB")).toBeInTheDocument();
    expect(screen.getByText("llm_unavailable")).toBeInTheDocument();
    expect(screen.queryByText("扫描命中率")).not.toBeInTheDocument();
  });
});

import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, beforeEach, afterEach, vi } from "vitest";
import EvolutionFeature from "../../../features/evolution";

// EvolutionFeature 直接委托 EvolutionCenterTab，后者挂载即 GET /api/evolution/runtime-flag
// 单数据源判定运维硬锁/总开关态（不再取 /api/health）。
// 走本地 fetch，不依赖任何 store。断言新视觉壳 + 开启/env 硬锁两条路径的真实 DOM。
const realFetch = globalThis.fetch;

describe("EvolutionFeature", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });
  afterEach(() => {
    globalThis.fetch = realFetch;
  });

  it("演化中心开启时渲染聚合卡与候选列表区", async () => {
    globalThis.fetch = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes("/api/evolution/runtime-flag")) {
        return {
          ok: true,
          json: async () => ({ envEvolutionEnabled: true, flag: { enabled: true, rolloutPercent: 100 } }),
        } as Response;
      }
      // /api/evolution/experiments
      return { ok: true, json: async () => ({ items: [] }) } as Response;
    }) as typeof fetch;

    render(<EvolutionFeature />);

    expect(screen.getByText("实验信封 · 候选 · 影子评测")).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByTestId("evolution-center")).toBeInTheDocument();
    });
    expect(screen.getByTestId("agg-experiments")).toBeInTheDocument();
    expect(screen.getByTestId("agg-significance")).toBeInTheDocument();
    expect(screen.getByTestId("proposal-list-empty")).toBeInTheDocument();
  });

  it("env 硬锁定时渲染锁定占位", async () => {
    globalThis.fetch = vi.fn(async () =>
      ({ ok: true, json: async () => ({ envEvolutionEnabled: false, flag: null }) }) as Response
    ) as typeof fetch;

    render(<EvolutionFeature />);

    await waitFor(() => {
      expect(screen.getByTestId("evolution-disabled")).toBeInTheDocument();
    });
  });
});

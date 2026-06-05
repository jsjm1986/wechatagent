import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, beforeEach, afterEach, vi } from "vitest";
import EvolutionFeature from "../../../features/evolution";

// EvolutionFeature 自取 /api/health 判定演化器是否启用，再委托 EvolutionCenterTab。
// 走本地 fetch，不依赖任何 store。断言新视觉壳 + 启用/未启用两条路径的真实 DOM。
const realFetch = globalThis.fetch;

describe("EvolutionFeature", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });
  afterEach(() => {
    globalThis.fetch = realFetch;
  });

  it("演化器启用时渲染聚合卡与候选列表区", async () => {
    globalThis.fetch = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes("/api/health")) {
        return { ok: true, json: async () => ({ evolutionEnabled: true }) } as Response;
      }
      // /api/evolution/experiments
      return { ok: true, json: async () => ({ items: [] }) } as Response;
    }) as typeof fetch;

    render(<EvolutionFeature />);

    // 面板级小标题（Shell 拥有大页头）
    expect(screen.getByText("实验信封 · 候选 · Shadow 评测")).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByTestId("evolution-center")).toBeInTheDocument();
    });
    // 聚合卡来自真实组件
    expect(screen.getByTestId("agg-experiments")).toBeInTheDocument();
    expect(screen.getByTestId("agg-significance")).toBeInTheDocument();
    // 无候选时的空态
    expect(screen.getByTestId("proposal-list-empty")).toBeInTheDocument();
  });

  it("演化器未启用时渲染禁用占位", async () => {
    globalThis.fetch = vi.fn(async () =>
      ({ ok: true, json: async () => ({ evolutionEnabled: false }) }) as Response
    ) as typeof fetch;

    render(<EvolutionFeature />);

    await waitFor(() => {
      expect(screen.getByTestId("evolution-disabled")).toBeInTheDocument();
    });
  });
});

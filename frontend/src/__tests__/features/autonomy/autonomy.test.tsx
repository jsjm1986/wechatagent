import { render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import AutonomyFeature, { AutonomyOutcomesTab } from "../../../features/autonomy";

// autonomy 频道一体化迁移后的视觉/集成测试（追加，不改既有 AutonomyOutcomesTab.test.tsx）。
// 验证：(1) 自包含 AutonomyFeature 渲染面板小标题 + 经 accountStore 选中账号驱动取数；
//       (2) CSS Module 视觉壳下，核心指标/AI 暂缓三类/发送链路真实 DOM 仍正确。
// 文案严守 AI 自主语义（AI 策略主动暂缓 / 安全门拦截 / AI 等待更多上下文）。

vi.mock("../../../stores/accountStore", () => ({
  useAccountStore: (selector: (s: unknown) => unknown) => {
    const state = {
      accounts: [{ accountId: "acc-1", nickname: "测试号" }],
      selectedAccountId: "acc-1",
      currentAccountId: () => "acc-1",
    };
    return typeof selector === "function" ? selector(state) : state;
  },
}));

const realFetch = globalThis.fetch;

type Metrics = Record<string, unknown>;

function metricsBody(over: Partial<Metrics> = {}): Metrics {
  return {
    horizon: "24h",
    accountId: "acc-1",
    totalRuns: 5,
    legacyModeUnchecked: 2,
    metrics: {
      revisionTriggerRate: 0.4,
      revisionPassRate: 0.5,
      aiHoldBreakdown: {
        heldByAiPolicy: 1 / 3,
        blockedBySafetyGuard: 1 / 3,
        aiWaitingForMoreContext: 1 / 3,
      },
      taxonomyCandidateRate: null,
      unverifiedClaimBlockRate: null,
      selfCritiqueAddressedRate: null,
      autonomyModeDistribution: { auto: null, assisted: null, blocked: null },
    },
    rawCounts: {
      totalRuns: 5,
      revisionApplied: 2,
      revisionPass: 1,
      heldByAiPolicy: 1,
      blockedBySafetyGuard: 1,
      aiWaitingForMoreContext: 1,
      taxonomyCandidate: 0,
      unverifiedClaimBlock: 0,
      selfCritiqueAddressed: 0,
      autonomyAuto: 0,
      autonomyAssisted: 0,
      autonomyBlocked: 0,
      legacyModeUnchecked: 2,
    },
    outboxLink: {
      totalEnqueued: 4,
      sent: 3,
      canceled: 1,
      failedTerminal: 0,
      sendSuccessRate: 0.75,
      canceledRate: 0.25,
      failedTerminalRate: 0,
    },
    ...over,
  };
}

function installFetch(metrics: Metrics) {
  globalThis.fetch = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    const body = url.includes("/revisions") ? { items: [] } : metrics;
    return {
      ok: true,
      status: 200,
      async json() {
        return body;
      },
      async text() {
        return JSON.stringify(body);
      },
    } as unknown as Response;
  }) as typeof fetch;
}

describe("AutonomyFeature — 一体化频道（新视觉壳）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });
  afterEach(() => {
    globalThis.fetch = realFetch;
  });

  it("渲染面板小标题，并由 accountStore 选中账号驱动取数", async () => {
    installFetch(metricsBody());
    render(<AutonomyFeature />);

    // 面板级小标题（Shell 拥有大页头 eyebrow/title/subtitle）
    expect(screen.getByText("修订 · AI 暂缓 · 发送链路 · Planner")).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByText("40.0%")).toBeInTheDocument();
    });
    // AI 暂缓三类标签 + 发送链路区
    expect(screen.getByText("AI 策略主动暂缓")).toBeInTheDocument();
    expect(screen.getByText("安全门拦截")).toBeInTheDocument();
    expect(screen.getByText("AI 等待更多上下文")).toBeInTheDocument();
    expect(screen.getByText("发送链路状态")).toBeInTheDocument();
  });

  it("AI 暂缓三类各 1 条 → 渲染 '33.3%（1 条）' 三处", async () => {
    installFetch(metricsBody());
    render(<AutonomyOutcomesTab accountId="acc-1" />);

    await waitFor(() => {
      expect(screen.getByText(/AI 策略主动暂缓/)).toBeInTheDocument();
    });
    expect(screen.getAllByText(/33\.3%（1 条）/)).toHaveLength(3);
  });
});

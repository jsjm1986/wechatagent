// W6 / Task 7.3：自治回路监控 Tab 端到端测试。
//
// 覆盖 4 个核心场景（与 `.kiro/specs/agent-autonomy-loop/tasks.md:388-393` 对齐，
// 与后端 `tests/outcomes_autonomy_endpoint.rs` 的 4 个集成测试一一镜像）：
//
// 1. totalRuns=0 → 所有比率渲染为 "—"
// 2. 5 runs / 2 revision → revisionTriggerRate 渲染 "40.0%"
// 3. 3 hold（每类 1 条）→ AI 暂缓三条 bar 渲染 "33.3%（1 条）"
// 4. held_for_human 历史脏值不进任何分类（aiHoldBreakdown 三类都 "0.0%"）
//
// 这是组件级 e2e：mock 全局 `fetch`，渲染 `AutonomyOutcomesTab`，等异步 fetch
// 完成后断言 DOM 文本。`accountId` 必须传，否则组件会短路在"请先选择账号"分支。

import { render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AutonomyOutcomesTab, formatRate } from "../App";

type PlannerSection = {
  silent: { tick: number; scanned: number; emitted: number; tickDetailEmitted: number; capped: number; backoff: number };
  commitment: { tick: number; overdueEmits: number; imminentEmits: number; backoff: number };
  stagnation: { tick: number; emitted: number; backoff: number };
};

type MetricsBody = {
  horizon: string;
  accountId: string;
  totalRuns: number;
  legacyModeUnchecked: number;
  metrics: {
    revisionTriggerRate: number | null;
    revisionPassRate: number | null;
    aiHoldBreakdown: {
      heldByAiPolicy: number | null;
      blockedBySafetyGuard: number | null;
      aiWaitingForMoreContext: number | null;
    };
    taxonomyCandidateRate: number | null;
    unverifiedClaimBlockRate: number | null;
    selfCritiqueAddressedRate: number | null;
    autonomyModeDistribution: { auto: number | null; assisted: number | null; blocked: number | null };
  };
  rawCounts: Record<string, number>;
  outboxLink: {
    totalEnqueued: number;
    sent: number;
    canceled: number;
    failedTerminal: number;
    sendSuccessRate: number | null;
    canceledRate: number | null;
    failedTerminalRate: number | null;
  };
  planner?: PlannerSection;
};

function emptyMetrics(): MetricsBody {
  return {
    horizon: "24h",
    accountId: "default",
    totalRuns: 0,
    legacyModeUnchecked: 0,
    metrics: {
      revisionTriggerRate: null,
      revisionPassRate: null,
      aiHoldBreakdown: {
        heldByAiPolicy: null,
        blockedBySafetyGuard: null,
        aiWaitingForMoreContext: null,
      },
      taxonomyCandidateRate: null,
      unverifiedClaimBlockRate: null,
      selfCritiqueAddressedRate: null,
      autonomyModeDistribution: { auto: null, assisted: null, blocked: null },
    },
    rawCounts: {
      totalRuns: 0,
      revisionApplied: 0,
      revisionPass: 0,
      heldByAiPolicy: 0,
      blockedBySafetyGuard: 0,
      aiWaitingForMoreContext: 0,
      taxonomyCandidate: 0,
      unverifiedClaimBlock: 0,
      selfCritiqueAddressed: 0,
      autonomyAuto: 0,
      autonomyAssisted: 0,
      autonomyBlocked: 0,
      legacyModeUnchecked: 0,
    },
    outboxLink: {
      totalEnqueued: 0,
      sent: 0,
      canceled: 0,
      failedTerminal: 0,
      sendSuccessRate: null,
      canceledRate: null,
      failedTerminalRate: null,
    },
  };
}

/** 安装 fetch mock，按 URL 子串匹配返回不同 body。 */
function installFetchMock(metrics: MetricsBody, revisions: unknown[] = []) {
  const fetchMock = vi.fn(async (url: string) => {
    const u = String(url);
    const body = u.includes("/revisions") ? { items: revisions } : metrics;
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
  });
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

describe("formatRate (helper)", () => {
  it("null → '—'", () => {
    expect(formatRate(null)).toBe("—");
  });
  it("0.4 → '40.0%'", () => {
    expect(formatRate(0.4)).toBe("40.0%");
  });
});

describe("AutonomyOutcomesTab — 自治回路监控 Tab", () => {
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("Case 1: totalRuns=0 → 所有比率渲染为 '—'", async () => {
    installFetchMock(emptyMetrics());
    render(<AutonomyOutcomesTab accountId="default" />);

    // 等待异步 fetch resolve + setData 完成
    await waitFor(() => {
      expect(screen.getByText(/升级后 run 数/)).toBeInTheDocument();
    });

    // 7 个 metric card（revisionTrigger / revisionPass / unverifiedClaim /
    // taxonomyCandidate / selfCritiqueAddressed / autonomyAuto / assisted-blocked）
    // + 3 个 outbox 单元格（sendSuccessRate / canceledRate / failedTerminalRate）
    // = 10 个独立 "—" 文本节点。HoldBar 三条 bar 的文本是 "—（0 条）"
    // 拼成单节点，不计入这里。
    const dashes = screen.getAllByText("—");
    expect(dashes.length).toBeGreaterThanOrEqual(10);

    // 单独断言三条 hold bar 也渲染为 dash + 0 条
    expect(screen.getAllByText(/—（0 条）/)).toHaveLength(3);
  });

  it("Case 2: 5 runs / 2 revision → revisionTriggerRate 渲染 '40.0%'", async () => {
    const m = emptyMetrics();
    m.totalRuns = 5;
    m.metrics.revisionTriggerRate = 0.4;
    m.metrics.revisionPassRate = 0.5;
    m.rawCounts.totalRuns = 5;
    m.rawCounts.revisionApplied = 2;
    m.rawCounts.revisionPass = 1;
    installFetchMock(m);
    render(<AutonomyOutcomesTab accountId="default" />);

    await waitFor(() => {
      expect(screen.getByText("40.0%")).toBeInTheDocument();
    });
    expect(screen.getByText("50.0%")).toBeInTheDocument(); // revisionPassRate

    // hint 行: 2/5 与 1/2 同时出现
    expect(screen.getByText("2/5")).toBeInTheDocument();
    expect(screen.getByText("1/2")).toBeInTheDocument();
  });

  it("Case 3: 三类 hold 各 1 条 → 三条 bar 渲染 '33.3%（1 条）'", async () => {
    const m = emptyMetrics();
    m.totalRuns = 3;
    m.rawCounts.totalRuns = 3;
    const oneThird = 1 / 3;
    m.metrics.aiHoldBreakdown = {
      heldByAiPolicy: oneThird,
      blockedBySafetyGuard: oneThird,
      aiWaitingForMoreContext: oneThird,
    };
    m.rawCounts.heldByAiPolicy = 1;
    m.rawCounts.blockedBySafetyGuard = 1;
    m.rawCounts.aiWaitingForMoreContext = 1;
    installFetchMock(m);
    render(<AutonomyOutcomesTab accountId="default" />);

    await waitFor(() => {
      expect(screen.getByText(/AI 策略主动暂缓/)).toBeInTheDocument();
    });
    // formatRate(1/3) = "33.3%"；HoldBar 把 "33.3%（1 条）" 拼到一个文本节点里。
    const triplets = screen.getAllByText(/33\.3%（1 条）/);
    expect(triplets).toHaveLength(3);

    // 三个固定标签都在
    expect(screen.getByText("AI 策略主动暂缓")).toBeInTheDocument();
    expect(screen.getByText("安全门拦截")).toBeInTheDocument();
    expect(screen.getByText("AI 等待更多上下文")).toBeInTheDocument();
  });

  it("Case 4: held_for_human 历史脏值不进任何分类 → 三条 bar 全部 '0.0%（0 条）'", async () => {
    // 后端 R10 已剔除：totalRuns 只算干净行，breakdown 三类都是 0/total。
    const m = emptyMetrics();
    m.totalRuns = 1;
    m.rawCounts.totalRuns = 1;
    m.metrics.aiHoldBreakdown = {
      heldByAiPolicy: 0,
      blockedBySafetyGuard: 0,
      aiWaitingForMoreContext: 0,
    };
    installFetchMock(m);
    render(<AutonomyOutcomesTab accountId="default" />);

    await waitFor(() => {
      expect(screen.getByText(/AI 策略主动暂缓/)).toBeInTheDocument();
    });

    const zeroBars = screen.getAllByText(/0\.0%（0 条）/);
    expect(zeroBars).toHaveLength(3);

    // legacyModeUnchecked 单独计数：rawCount 行可能是 0 也行；要点是它不污染上面三类。
    expect(screen.getByText(/未升级 run/)).toBeInTheDocument();
  });

  // M3 / Task 72：Planner section 渲染。
  // 镜像后端 `tests/outcomes_autonomy_endpoint.rs` 对 `planner` 子段的断言：
  // silent / commitment / stagnation 三 column 各自数字正确，且 backoff 计数会
  // 反映到 hint 行，证明三段独立可见。
  it("Case 5: response 带 planner 子段 → 三 column 计数与 hint 渲染正确", async () => {
    const m = emptyMetrics();
    m.planner = {
      silent: { tick: 12, scanned: 30, emitted: 5, tickDetailEmitted: 5, capped: 1, backoff: 2 },
      commitment: { tick: 12, overdueEmits: 3, imminentEmits: 4, backoff: 1 },
      stagnation: { tick: 12, emitted: 2, backoff: 0 },
    };
    installFetchMock(m);
    render(<AutonomyOutcomesTab accountId="default" />);

    await waitFor(() => {
      expect(screen.getByTestId("planner-section")).toBeInTheDocument();
    });

    // 三 column 都在
    const silent = screen.getByTestId("planner-silent");
    const commitment = screen.getByTestId("planner-commitment");
    const stagnation = screen.getByTestId("planner-stagnation");

    // silent.emitted=5 / commitment.overdue+imminent=7 / stagnation.emitted=2
    expect(silent.querySelector(".autonomyMetricValue")?.textContent).toBe("5");
    expect(commitment.querySelector(".autonomyMetricValue")?.textContent).toBe("7");
    expect(stagnation.querySelector(".autonomyMetricValue")?.textContent).toBe("2");

    // hint 行中包含 backoff / capped / tick 等 raw count
    expect(silent.textContent).toMatch(/tick 12/);
    expect(silent.textContent).toMatch(/scanned 30/);
    expect(silent.textContent).toMatch(/capped 1/);
    expect(silent.textContent).toMatch(/backoff 2/);

    expect(commitment.textContent).toMatch(/overdue 3/);
    expect(commitment.textContent).toMatch(/imminent 4/);
    expect(commitment.textContent).toMatch(/backoff 1/);

    expect(stagnation.textContent).toMatch(/backoff 0/);
  });

  // M3 / Task 72：响应未带 planner 子段时（兼容旧后端）整段不渲染，且不抛错。
  it("Case 6: response 不带 planner 子段 → planner section 不渲染", async () => {
    const m = emptyMetrics();
    installFetchMock(m);
    render(<AutonomyOutcomesTab accountId="default" />);

    await waitFor(() => {
      expect(screen.getByText(/升级后 run 数/)).toBeInTheDocument();
    });
    expect(screen.queryByTestId("planner-section")).toBeNull();
  });
});

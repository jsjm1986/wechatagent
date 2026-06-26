import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { ProposalReleaseCard } from "../../../components/review/ProposalReleaseCard";

// ProposalReleaseCard 自身拉详情（GET /api/evolution/proposals/:id → ProposalDetailResponse）。
// 取值路径有别（proposalTypes.ts 实证）：diffSummary/riskNote/previousPromptVersion/evalMetrics
// 在 data.proposal.*；cohortRunIds 在 data 顶层（ProposalDetailResponse:117）。
// 组件 load 异步，断言用 findBy*。本文件只覆盖新增的 5 字段元数据渲染与空值处理。

const getMock = vi.fn();

vi.mock("../../../lib/api", () => ({
  api: {
    get: (...args: unknown[]) => getMock(...args),
    post: vi.fn().mockResolvedValue({}),
  },
}));

function baseDetail(overrides: Record<string, unknown> = {}) {
  return {
    proposal: {
      id: "PR1",
      experimentId: "EXP1",
      workspaceId: "W1",
      accountId: "A1",
      kind: "prompt",
      status: "eligible_for_release",
      gateKey: null,
      currentValue: null,
      proposedValue: null,
      cohortNotes: {},
      proposedTemplateKey: "soul",
      proposedSection: "tone",
      diffSummary: "收紧追问节奏，避免连续提问。",
      diffSnippet: "新内容片段",
      criticReasoning: null,
      expectedImprovementOn: null,
      riskNote: "存在轻微语气偏冷风险。",
      previousPromptVersion: "v2.3.1",
      evalMetrics: { win_rate: 0.62, sample_size: 48 },
      evalReplaysCompleted: 10,
      evalReplaysFailed: 0,
      significancePassed: true,
      failureReason: null,
      releasedAt: null,
      releasedBy: null,
      rolledBackAt: null,
      rolledBackBy: null,
      createdAt: "2026-06-01T00:00:00Z",
      updatedAt: "2026-06-02T00:00:00Z",
      ...overrides,
    },
    experiment: null,
    cohortRunIds: ["run-aaa", "run-bbb"],
    shadowReplays: { totalCompleted: 10, totalFailed: 0, samples: [] },
    currentState: {},
  };
}

function renderCard() {
  return render(<ProposalReleaseCard proposalId="PR1" />);
}

describe("ProposalReleaseCard 元数据 5 字段渲染（E12）", () => {
  beforeEach(() => vi.clearAllMocks());

  it("renders riskNote/diffSummary/evalMetrics/cohortRunIds/previousPromptVersion", async () => {
    getMock.mockResolvedValue(baseDetail());
    renderCard();

    // diffSummary（data.proposal.diffSummary）
    const diffSummary = await screen.findByTestId("proposal-diff-summary");
    expect(diffSummary).toHaveTextContent("收紧追问节奏，避免连续提问。");
    // riskNote（data.proposal.riskNote）
    expect(screen.getByTestId("proposal-risk-note")).toHaveTextContent(
      "存在轻微语气偏冷风险。",
    );
    // previousPromptVersion（data.proposal.previousPromptVersion）
    expect(screen.getByTestId("proposal-prev-version")).toHaveTextContent("v2.3.1");
    // evalMetrics（data.proposal.evalMetrics，Record key-value）
    const metrics = screen.getByTestId("proposal-eval-metrics");
    expect(metrics).toHaveTextContent("win_rate");
    expect(metrics).toHaveTextContent("0.62");
    expect(metrics).toHaveTextContent("sample_size");
    // cohortRunIds（data 顶层，数组）
    const cohort = screen.getByTestId("proposal-cohort-runs");
    expect(cohort).toHaveTextContent("run-aaa");
    expect(cohort).toHaveTextContent("run-bbb");
  });

  it("5 字段全空（null/{}/[]）时对应区块不渲染", async () => {
    getMock.mockResolvedValue(
      baseDetail({
        diffSummary: null,
        riskNote: null,
        previousPromptVersion: null,
        evalMetrics: {},
      }),
    );
    // cohortRunIds 顶层置空
    getMock.mockResolvedValue({
      ...baseDetail({
        diffSummary: null,
        riskNote: null,
        previousPromptVersion: null,
        evalMetrics: {},
      }),
      cohortRunIds: [],
    });
    renderCard();

    // 等组件加载完成（详情容器出现）
    await screen.findByTestId("proposal-detail");
    await waitFor(() => expect(getMock).toHaveBeenCalled());

    expect(screen.queryByTestId("proposal-diff-summary")).not.toBeInTheDocument();
    expect(screen.queryByTestId("proposal-risk-note")).not.toBeInTheDocument();
    expect(screen.queryByTestId("proposal-prev-version")).not.toBeInTheDocument();
    expect(screen.queryByTestId("proposal-eval-metrics")).not.toBeInTheDocument();
    expect(screen.queryByTestId("proposal-cohort-runs")).not.toBeInTheDocument();
  });
});

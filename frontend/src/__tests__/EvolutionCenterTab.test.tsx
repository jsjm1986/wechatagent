// agent-self-evolution M4 W4 Task 5.10：演化中心 Tab 单元测试。
//
// 覆盖 ≥4 个核心场景（与 .kiro/specs/agent-self-evolution/tasks.md 5.10 对齐，
// 与 EvolutionCenterTab.tsx 的可见接口一一镜像）：
//
//   1) ProposalList 渲染 4 种 status 徽章（eligibleForRelease / released /
//      rolledBack / rejectedBelowThreshold）— 每个徽章 tone class 与文案正确
//   2) [发布] 按钮只在 status === 'eligible_for_release' 启用；其余状态置灰
//   3) ReleaseModal：输入错误串（小写 / 含尾空格 / WRONG）时 [确认发布] 仍 disabled
//      且不发 POST 请求
//   4) Prompt diff 双栏：current 与 proposed 文本各落在自己的 testid 栏，不串栏
//
// 渲染方式：mock 全局 fetch，给 /api/evolution/experiments 与
// /api/evolution/proposals/:id 返回 fixture body，等异步完成后断言 DOM。

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  ConfirmModal,
  EvolutionCenterTab,
  type ExperimentItem,
  type ProposalDetailResponse,
  type ProposalSummary,
} from "../EvolutionCenterTab";

function makeProposal(over: Partial<ProposalSummary>): ProposalSummary {
  return {
    id: "00000000000000000000abcd",
    kind: "threshold",
    status: "pending_eval",
    gateKey: "fact_risk_block",
    proposedTemplateKey: null,
    proposedSection: null,
    currentValue: 6,
    proposedValue: 7,
    significancePassed: null,
    evalReplaysCompleted: 0,
    evalReplaysFailed: 0,
    failureReason: null,
    createdAt: new Date(Date.now() - 60_000).toISOString(),
    updatedAt: new Date(Date.now() - 60_000).toISOString(),
    ...over,
  };
}

function makeExperimentItem(proposals: ProposalSummary[]): ExperimentItem {
  const startedAt = new Date(Date.now() - 60_000).toISOString();
  return {
    experiment: {
      experimentId: "exp_default_1",
      workspaceId: "default",
      accountId: "default",
      status: "awaiting_admin",
      windowHours: 24,
      startedAt,
      updatedAt: startedAt,
      finishedAt: null,
      cohortThresholdSize: 1,
      cohortPromptSize: 0,
      budgetUsedTokens: 0,
      budgetUsedCalls: 0,
      proposalsCount: proposals.length,
      proposalsEligibleCount: proposals.filter((p) => p.status === "eligible_for_release").length,
    },
    proposalsCounts: {},
    proposals,
  };
}

function makeDetail(over: Partial<ProposalDetailResponse["proposal"]>): ProposalDetailResponse {
  return {
    proposal: {
      id: "00000000000000000000abcd",
      experimentId: "exp_default_1",
      workspaceId: "default",
      accountId: "default",
      kind: "threshold",
      status: "eligible_for_release",
      gateKey: "fact_risk_block",
      currentValue: 6,
      proposedValue: 7,
      cohortNotes: { hit_rate_observed: 0.18 },
      proposedTemplateKey: null,
      proposedSection: null,
      diffSummary: null,
      diffSnippet: null,
      criticReasoning: null,
      expectedImprovementOn: null,
      riskNote: null,
      previousPromptVersion: null,
      evalMetrics: {},
      evalReplaysCompleted: 30,
      evalReplaysFailed: 0,
      significancePassed: true,
      failureReason: null,
      releasedAt: null,
      releasedBy: null,
      rolledBackAt: null,
      rolledBackBy: null,
      createdAt: new Date(Date.now() - 60_000).toISOString(),
      updatedAt: new Date(Date.now() - 60_000).toISOString(),
      ...over,
    },
    experiment: null,
    cohortRunIds: [],
    shadowReplays: { totalCompleted: 30, totalFailed: 0, samples: [] },
    currentState: { kind: "threshold", currentValue: 6 },
  };
}

describe("EvolutionCenterTab", () => {
  let fetchMock: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.clearAllMocks();
  });

  it("renders status badges for the four W4 statuses", async () => {
    const proposals = [
      makeProposal({ id: "1".repeat(24), status: "eligible_for_release" }),
      makeProposal({ id: "2".repeat(24), status: "released" }),
      makeProposal({ id: "3".repeat(24), status: "rolled_back" }),
      makeProposal({ id: "4".repeat(24), status: "rejected_below_threshold" }),
    ];
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ items: [makeExperimentItem(proposals)] }),
    });

    render(<EvolutionCenterTab enabled={true} />);

    await waitFor(() => {
      expect(screen.getByTestId("status-badge-eligible_for_release")).toHaveTextContent("可发布");
    });
    expect(screen.getByTestId("status-badge-released")).toHaveTextContent("已发布");
    expect(screen.getByTestId("status-badge-rolled_back")).toHaveTextContent("已回滚");
    expect(screen.getByTestId("status-badge-rejected_below_threshold")).toHaveTextContent("未达标");

    expect(
      screen.getByTestId("status-badge-eligible_for_release").getAttribute("data-tone"),
    ).toBe("success");
    expect(screen.getByTestId("status-badge-released").getAttribute("data-tone")).toBe("primary");
    expect(screen.getByTestId("status-badge-rolled_back").getAttribute("data-tone")).toBe("danger");
    expect(
      screen.getByTestId("status-badge-rejected_below_threshold").getAttribute("data-tone"),
    ).toBe("warn");
  });

  it("enables the release button only for eligible_for_release proposals", async () => {
    // 列表里同时存在两条候选：1) eligible_for_release 2) released
    const eligibleProposal = makeProposal({
      id: "a".repeat(24),
      status: "eligible_for_release",
    });
    const releasedProposal = makeProposal({
      id: "b".repeat(24),
      status: "released",
    });
    // 列表 fetch
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ items: [makeExperimentItem([eligibleProposal, releasedProposal])] }),
    });
    // 第一次详情 fetch（点 eligible 行）
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => makeDetail({ id: "a".repeat(24), status: "eligible_for_release" }),
    });
    // 第二次详情 fetch（点 released 行）
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => makeDetail({ id: "b".repeat(24), status: "released" }),
    });

    render(<EvolutionCenterTab enabled={true} />);
    await waitFor(() => screen.getByTestId(`proposal-row-${"a".repeat(24)}`));

    fireEvent.click(screen.getByTestId(`proposal-row-${"a".repeat(24)}`));
    await waitFor(() => screen.getByTestId("release-button"));
    expect(screen.getByTestId("release-button")).not.toBeDisabled();
    expect(screen.getByTestId("rollback-button")).toBeDisabled();

    // 切到 released 那条
    fireEvent.click(screen.getByText("关闭"));
    fireEvent.click(screen.getByTestId(`proposal-row-${"b".repeat(24)}`));
    await waitFor(() => screen.getByTestId("release-button"));
    expect(screen.getByTestId("release-button")).toBeDisabled();
    expect(screen.getByTestId("rollback-button")).not.toBeDisabled();
  });

  it("ConfirmModal blocks submission unless the literal matches exactly", async () => {
    const onDone = vi.fn();
    const onClose = vi.fn();
    render(
      <ConfirmModal
        kind="release"
        proposalId={"a".repeat(24)}
        onClose={onClose}
        onDone={onDone}
      />,
    );

    const input = screen.getByTestId("confirm-input-release") as HTMLInputElement;
    const submit = screen.getByTestId("confirm-submit-release") as HTMLButtonElement;

    // 默认 disabled
    expect(submit.disabled).toBe(true);

    // 小写不匹配
    fireEvent.change(input, { target: { value: "release" } });
    expect(submit.disabled).toBe(true);

    // 完全错的串
    fireEvent.change(input, { target: { value: "WRONG" } });
    expect(submit.disabled).toBe(true);

    // 尾随空格不匹配
    fireEvent.change(input, { target: { value: "RELEASE " } });
    expect(submit.disabled).toBe(true);

    // 这一段都不能触发请求
    fireEvent.click(submit);
    expect(fetchMock).not.toHaveBeenCalled();

    // 完全匹配后启用
    fireEvent.change(input, { target: { value: "RELEASE" } });
    expect(submit.disabled).toBe(false);
  });

  it("renders prompt diff in two distinct panes without bleed-through", async () => {
    const promptProposal = makeProposal({
      id: "c".repeat(24),
      kind: "prompt",
      status: "eligible_for_release",
      gateKey: null,
      proposedTemplateKey: "user_ops/system_contract",
      proposedSection: "fact_risk_block",
      currentValue: null,
      proposedValue: null,
    });
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ items: [makeExperimentItem([promptProposal])] }),
    });
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () =>
        ({
          proposal: {
            id: "c".repeat(24),
            experimentId: "exp_default_1",
            workspaceId: "default",
            accountId: "default",
            kind: "prompt",
            status: "eligible_for_release",
            gateKey: null,
            currentValue: null,
            proposedValue: null,
            cohortNotes: {},
            proposedTemplateKey: "user_ops/system_contract",
            proposedSection: "fact_risk_block",
            diffSummary: null,
            diffSnippet: "PROPOSED-NEW-PROMPT-BODY",
            criticReasoning: "make claims more verifiable",
            expectedImprovementOn: ["fact_risk_block", "product_accuracy_score_block"],
            riskNote: null,
            previousPromptVersion: null,
            evalMetrics: {},
            evalReplaysCompleted: 30,
            evalReplaysFailed: 0,
            significancePassed: true,
            failureReason: null,
            releasedAt: null,
            releasedBy: null,
            rolledBackAt: null,
            rolledBackBy: null,
            createdAt: new Date().toISOString(),
            updatedAt: new Date().toISOString(),
          },
          experiment: null,
          cohortRunIds: [],
          shadowReplays: { totalCompleted: 30, totalFailed: 0, samples: [] },
          currentState: { currentSectionText: "CURRENT-EXISTING-PROMPT-BODY" },
        }) satisfies ProposalDetailResponse,
    });

    render(<EvolutionCenterTab enabled={true} />);
    await waitFor(() => screen.getByTestId(`proposal-row-${"c".repeat(24)}`));
    fireEvent.click(screen.getByTestId(`proposal-row-${"c".repeat(24)}`));
    await waitFor(() => screen.getByTestId("prompt-diff"));

    const currentPane = screen.getByTestId("prompt-diff-current");
    const proposedPane = screen.getByTestId("prompt-diff-proposed");
    expect(currentPane.textContent).toContain("CURRENT-EXISTING-PROMPT-BODY");
    expect(currentPane.textContent).not.toContain("PROPOSED-NEW-PROMPT-BODY");
    expect(proposedPane.textContent).toContain("PROPOSED-NEW-PROMPT-BODY");
    expect(proposedPane.textContent).not.toContain("CURRENT-EXISTING-PROMPT-BODY");

    expect(screen.getByTestId("critic-reasoning").textContent).toContain("make claims more verifiable");
    expect(screen.getByTestId("expected-improvement").textContent).toContain("fact_risk_block");
    expect(screen.getByTestId("expected-improvement").textContent).toContain(
      "product_accuracy_score_block",
    );
  });
});

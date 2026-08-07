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

function makeExperimentsResponse(items: ExperimentItem[]) {
  const proposals = items.flatMap((item) => item.proposals);
  const evaluated = proposals.filter((proposal) => proposal.significancePassed !== null);
  const passed = evaluated.filter((proposal) => proposal.significancePassed === true).length;
  const now = new Date().toISOString();
  return {
    items,
    aggregate7d: {
      experiments: items.length,
      proposals: proposals.length,
      released: proposals.filter((proposal) => proposal.status === "released").length,
      rolledBack: proposals.filter((proposal) => proposal.status === "rolled_back").length,
      significancePassRate: evaluated.length === 0 ? null : passed / evaluated.length,
      coverage: {
        complete: true,
        source: "server_time_window",
        windowHours: 168,
        windowStart: new Date(Date.now() - 7 * 24 * 60 * 60 * 1000).toISOString(),
        windowEnd: now,
        asOf: now,
        experimentsScanned: items.length,
      },
    },
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

  // Task 3 三态重构后，Tab 挂载即先 GET /api/evolution/runtime-flag（loadFlag，第一个 fetch），
  // 再按 flag 启用与否决定是否 GET experiments（第二个 fetch）。既有用例的顺序型
  // mockResolvedValueOnce 队列原本第一个是 experiments，现在全部错位——故每个用例须在
  // experiments mock 之前插一个 runtime-flag mock。默认 env 允许 + flag 开 + 100% 全量。
  function mockRuntimeFlag(over?: { envEvolutionEnabled?: boolean; enabled?: boolean; rolloutPercent?: number }) {
    return {
      ok: true,
      json: async () => ({
        envEvolutionEnabled: over?.envEvolutionEnabled ?? true,
        flag: { enabled: over?.enabled ?? true, rolloutPercent: over?.rolloutPercent ?? 100 },
      }),
    };
  }

  it("renders status badges for the four W4 statuses", async () => {
    const proposals = [
      makeProposal({ id: "1".repeat(24), status: "eligible_for_release" }),
      makeProposal({ id: "2".repeat(24), status: "released" }),
      makeProposal({ id: "3".repeat(24), status: "rolled_back" }),
      makeProposal({ id: "4".repeat(24), status: "rejected_below_threshold" }),
    ];
    fetchMock.mockResolvedValueOnce(mockRuntimeFlag());
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => makeExperimentsResponse([makeExperimentItem(proposals)]),
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
    fetchMock.mockResolvedValueOnce(mockRuntimeFlag());
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () =>
        makeExperimentsResponse([makeExperimentItem([eligibleProposal, releasedProposal])]),
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
    fetchMock.mockResolvedValueOnce(mockRuntimeFlag());
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => makeExperimentsResponse([makeExperimentItem([promptProposal])]),
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

  // 前后端对齐批次1 Task 5（C3）：runtime-flag 灰度开关 + rollout 比例控件。
  // 本文件 mock 全局 fetch（非 lib/api 的 api.put），故断言落在 fetch 调用上：
  // 找出 method === "PUT" 的那次 fetch，校验 URL 与 camelCase body。
  it("渲染 runtime-flag 灰度控件并 PUT 保存", async () => {
    // Task 3 后 Tab 挂载先 loadFlag(call#1)；flag 开 → 触发 experiments(call#2)。
    // 高级灰度比例改 50 后点「保存灰度」走 saveFlag → PUT(call#3)。
    fetchMock.mockResolvedValueOnce(mockRuntimeFlag({ enabled: true, rolloutPercent: 100 }));
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => makeExperimentsResponse([]),
    });
    // PUT 响应（保存灰度后回写）。复刻后端真实结构：{ ok, flag: { 内层 } }。
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        ok: true,
        flag: {
          enabled: true,
          rolloutPercent: 50,
          rolloutPercentRaw: 50,
          thresholdAutoReleaseEnabled: null,
          updatedBy: "admin",
          updatedAt: "2026-06-26T00:00:00Z",
        },
      }),
    });

    render(<EvolutionCenterTab enabled={true} />);

    const input = (await screen.findByLabelText(/灰度比例/)) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "50" } });

    fireEvent.click(screen.getByText("保存灰度"));

    await waitFor(() => {
      const putCall = fetchMock.mock.calls.find(
        ([, opts]) => opts && (opts as RequestInit).method === "PUT",
      );
      expect(putCall).toBeTruthy();
      expect(putCall![0]).toBe("/api/evolution/runtime-flag");
      const body = JSON.parse((putCall![1] as RequestInit).body as string);
      expect(body).toMatchObject({ enabled: true, rolloutPercent: 50 });
    });
    // 保存后读回展示值来自 .flag 内层（rolloutPercent: 50）。
    await waitFor(() => {
      expect((screen.getByLabelText(/灰度比例/) as HTMLInputElement).value).toBe("50");
    });
  });

  it("读取当前配置按钮 GET runtime-flag 并回填", async () => {
    // 挂载 loadFlag(call#1)：env 允许 + flag 开 + 100% → 触发 experiments(call#2)。
    fetchMock.mockResolvedValueOnce(mockRuntimeFlag({ enabled: true, rolloutPercent: 100 }));
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => makeExperimentsResponse([]),
    });
    // 「读取当前配置」按钮再次 GET(call#3)。复刻后端真实结构：配置体在 .flag 子对象里
    //（外层还有 workspaceId / envEvolutionEnabled）。读回必须从 .flag 内层取，否则恒显 0。
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        workspaceId: "default",
        envEvolutionEnabled: true,
        flag: {
          enabled: true,
          rolloutPercent: 30,
          rolloutPercentRaw: 30,
          thresholdAutoReleaseEnabled: null,
          updatedBy: "admin",
          updatedAt: "2026-06-26T00:00:00Z",
        },
      }),
    });

    render(<EvolutionCenterTab enabled={true} />);

    fireEvent.click(await screen.findByText("读取当前配置"));

    await waitFor(() => {
      const input = screen.getByLabelText(/灰度比例/) as HTMLInputElement;
      expect(input.value).toBe("30");
    });
    // 同时断言总开关回填来自 .flag 内层（enabled: true）。
    expect((screen.getByText("演化中心总开关").closest("label")!.querySelector("input") as HTMLInputElement).checked).toBe(true);
  });

  // 前后端对齐批次1 Task 6（C4）：阈值变更不可变审计日志视图。
  // 按钮点击触发 GET /api/evolution/threshold-overrides/audit（与 runtime-flag
  // 一致的"点按钮加载"模式，不在挂载期自动 GET，避免打乱既有顺序型 mock 队列）。
  it("点按钮加载阈值变更审计日志并渲染行", async () => {
    // 挂载 loadFlag(call#1) → 触发 experiments(call#2)。
    fetchMock.mockResolvedValueOnce(mockRuntimeFlag());
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => makeExperimentsResponse([]),
    });
    // 审计 GET 返回一条 release 行（后端真实字段）。
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        items: [
          {
            id: "aud1",
            gateKey: "fact_risk_block",
            action: "release",
            previousValue: 6,
            newValue: 7,
            sourceProposalId: "00000000000000000000abcd",
            decidedBy: "admin",
            decidedAt: "2026-06-26T00:00:00Z",
            hitRateObserved: 0.18,
            significanceMetrics: null,
          },
        ],
      }),
    });

    render(<EvolutionCenterTab enabled={true} />);

    fireEvent.click(await screen.findByText(/阈值变更审计/));

    await waitFor(() => {
      const auditCall = fetchMock.mock.calls.find(
        ([url]) =>
          typeof url === "string" &&
          url.includes("/api/evolution/threshold-overrides/audit"),
      );
      expect(auditCall).toBeTruthy();
    });

    await waitFor(() => {
      expect(screen.getByTestId("threshold-audit-table")).toBeInTheDocument();
    });
    const table = screen.getByTestId("threshold-audit-table");
    expect(table.textContent).toContain("fact_risk_block");
    expect(table.textContent).toContain("release");
    expect(table.textContent).toContain("admin");
  });

  it("审计为空时显示暂无审计记录", async () => {
    fetchMock.mockResolvedValueOnce(mockRuntimeFlag());
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => makeExperimentsResponse([]),
    });
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ items: [] }),
    });

    render(<EvolutionCenterTab enabled={true} />);

    fireEvent.click(await screen.findByText(/阈值变更审计/));

    await waitFor(() => {
      expect(screen.getByTestId("threshold-audit-empty")).toBeInTheDocument();
    });
    expect(screen.getByTestId("threshold-audit-empty").textContent).toContain(
      "暂无审计记录",
    );
  });

  // ── Task 3 三态重构验证：运维硬锁 / env 允许但 flag 关 / 打开总开关=100% 全量 ──

  it("env 硬锁定（envEvolutionEnabled=false）渲染锁定占位", async () => {
    fetchMock.mockResolvedValueOnce(mockRuntimeFlag({ envEvolutionEnabled: false, enabled: false }));
    render(<EvolutionCenterTab enabled={true} />);
    await waitFor(() => {
      expect(screen.getByTestId("evolution-disabled")).toHaveTextContent("运维硬锁定");
    });
  });

  it("env 允许但 flag 关时仍加载历史实验并显示暂停提示条（F-008）", async () => {
    fetchMock.mockResolvedValueOnce(mockRuntimeFlag({ envEvolutionEnabled: true, enabled: false, rolloutPercent: 0 }));
    // F-008：flag 关时 load() 不再早返回，仍拉 experiments —— 关闭态只停产新实验，
    // 历史实验须可见，否则管理员误判"演化从未运行"。返回 1 条历史让提示条命中。
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () =>
        makeExperimentsResponse([
          makeExperimentItem([makeProposal({ id: "e".repeat(24), status: "released" })]),
        ]),
    });
    render(<EvolutionCenterTab enabled={true} />);
    await waitFor(() => {
      expect(screen.getByTestId("runtime-flag-panel")).toBeInTheDocument();
    });
    // flag 关 → 仍应发起 experiments 请求（F-008：不再早返回）。
    await waitFor(() => {
      const experimentsCall = fetchMock.mock.calls.find(([url]) =>
        String(url).includes("/api/evolution/experiments"),
      );
      expect(experimentsCall).toBeTruthy();
    });
    // 有历史实验时显示"已关闭 / 仍保留 N 条历史"暂停提示条。
    await waitFor(() => {
      expect(screen.getByTestId("evolution-dormant-notice")).toBeInTheDocument();
    });
    expect(screen.getByTestId("evolution-dormant-notice").textContent).toContain("已关闭");
    // 总开关 checkbox 可点（未 disabled）。
    const toggle = screen.getByText("演化中心总开关").closest("label")?.querySelector("input");
    expect(toggle).not.toBeNull();
    expect(toggle).not.toBeDisabled();
  });

  it("打开总开关 PUT 写 enabled:true + rolloutPercent:100", async () => {
    fetchMock.mockResolvedValueOnce(mockRuntimeFlag({ envEvolutionEnabled: true, enabled: false, rolloutPercent: 0 }));
    // 关闭态也先加载历史实验。
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => makeExperimentsResponse([]),
    });
    // PUT 权威回读。
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ ok: true, flag: { enabled: true, rolloutPercent: 100 } }),
    });
    // 权威回读使 flagEnabled→true 后再次刷新 experiments。
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => makeExperimentsResponse([]),
    });

    render(<EvolutionCenterTab enabled={true} />);
    await waitFor(() => screen.getByText("演化中心总开关"));
    const toggle = screen.getByText("演化中心总开关").closest("label")!.querySelector("input")!;
    fireEvent.click(toggle);

    await waitFor(() => {
      const putCall = fetchMock.mock.calls.find(
        (c) => String(c[0]).includes("/api/evolution/runtime-flag") && (c[1] as RequestInit)?.method === "PUT",
      );
      expect(putCall).toBeTruthy();
      const body = JSON.parse((putCall![1] as RequestInit).body as string);
      expect(body.enabled).toBe(true);
      expect(body.rolloutPercent).toBe(100);
    });
    await waitFor(() => expect(toggle).toBeChecked());
  });

  it("服务端未提供完整 7 天 coverage 时不从可见列表伪造指标", async () => {
    fetchMock.mockResolvedValueOnce(mockRuntimeFlag());
    fetchMock.mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        items: [makeExperimentItem([makeProposal({ status: "released" })])],
      }),
    });

    render(<EvolutionCenterTab enabled={true} />);

    await waitFor(() => {
      expect(screen.getByRole("alert")).toHaveTextContent("未返回完整的近 7 天统计覆盖");
    });
    expect(screen.getByTestId("agg-experiments")).toHaveTextContent("—");
    expect(screen.getByTestId("aggregate-coverage")).toHaveTextContent("尚未加载");
    expect(screen.queryByTestId(`proposal-row-${"0".repeat(20)}abcd`)).toBeNull();
  });

  it("总开关 PUT 失败时保持服务端原值并显示错误", async () => {
    fetchMock.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url === "/api/evolution/runtime-flag" && init?.method === "PUT") {
        return { ok: false, text: async () => "save failed" };
      }
      if (url === "/api/evolution/runtime-flag") {
        return mockRuntimeFlag({ envEvolutionEnabled: true, enabled: false, rolloutPercent: 0 });
      }
      if (url.startsWith("/api/evolution/experiments")) {
        return { ok: true, json: async () => makeExperimentsResponse([]) };
      }
      throw new Error(`unexpected fetch in test: ${url}`);
    });

    render(<EvolutionCenterTab enabled={true} />);
    const toggle = (await screen.findByText("演化中心总开关"))
      .closest("label")!
      .querySelector("input")!;
    expect(toggle).not.toBeChecked();

    fireEvent.click(toggle);

    await waitFor(() => {
      expect(screen.getByTestId("runtime-flag-msg")).toHaveTextContent("save failed");
    });
    expect(screen.getByTestId("runtime-flag-msg")).toHaveAttribute("role", "alert");
    expect(toggle).not.toBeChecked();
  });

  it("runtime-flag 拉取失败时显错误+重试,不卡在加载中", async () => {
    // 首次 GET 失败（非 ok）→ loadFlag catch → 错误态，而非永久 envAllowed=null 卡加载中。
    fetchMock.mockResolvedValueOnce({ ok: false, text: async () => "boom" });
    render(<EvolutionCenterTab enabled={true} />);
    await waitFor(() => {
      expect(screen.getByTestId("evolution-flag-error")).toBeInTheDocument();
    });
    expect(screen.getByRole("alert")).toHaveTextContent("boom");
    // 不应停在加载中占位
    expect(screen.queryByTestId("evolution-flag-loading")).toBeNull();
    // 有重试入口
    expect(screen.getByText("重试")).toBeInTheDocument();
  });
});

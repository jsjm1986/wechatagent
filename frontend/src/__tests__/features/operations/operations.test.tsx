import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import OperationsFeature, { formatScores } from "../../../features/operations";
import { useOperationsStore } from "../../../stores/operationsStore";
import { useAccountStore } from "../../../stores/accountStore";
import { api } from "../../../lib/api";

// Mock stores
vi.mock("../../../stores/operationsStore");
vi.mock("../../../stores/accountStore");

// Mock api（复核 / 取消按钮走 api.post）
vi.mock("../../../lib/api", () => ({
  api: { post: vi.fn().mockResolvedValue({ ok: true }), get: vi.fn() },
}));

// Mock fetch for API calls
(globalThis as any).fetch = vi.fn();

describe("OperationsFeature", () => {
  const mockLoadOperationsData = vi.fn();
  const mockCurrentAccountId = vi.fn();
  const mockReviewTaskNow = vi.fn();
  const mockCancelTask = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();

    // Mock fetch responses
    ((globalThis as any).fetch as any).mockResolvedValue({
      ok: true,
      json: () => Promise.resolve({ items: [] }),
    });

    // Mock store implementations
    (useOperationsStore as any).mockReturnValue({
      events: [],
      tasks: [{ id: "1", accountId: "test-account-id", content: "测试任务", status: "pending" }],
      decisionReviews: [],
      llmUsage: {
        asOf: "2026-07-20T00:00:00Z",
        window: { kind: "retained_logs", start: null, end: "2026-07-20T00:00:00Z" },
        summary: {
          totalCalls: 0,
          totalTokens: 0,
          promptCacheHitTokens: 0,
          promptCacheMissTokens: 0,
          promptCacheHitRate: 0,
          knownUsageCalls: 0,
          unknownUsageCalls: 0,
          usageComplete: true,
        },
        itemsReturned: 0,
        itemsLimit: 100,
        itemsTruncated: false,
        items: [],
      },
      opsTab: "tasks",
      dataAccountId: "test-account-id",
      setOpsTab: vi.fn(),
      loadOperationsData: mockLoadOperationsData,
      reviewTaskNow: mockReviewTaskNow,
      cancelTask: mockCancelTask,
    });

    // accountStore 既被 selector 形式订阅（effectiveAccountId 派生原始值），
    // 也被整对象解构使用——mock 同时支持两种调用。
    const accountState = {
      accounts: [
        { id: "test-account-id", accountId: "test-account-id", alias: "测试", displayName: "测试", online: true },
      ],
      selectedAccountId: "test-account-id",
      currentAccountId: mockCurrentAccountId,
    };
    (useAccountStore as any).mockImplementation((selector?: any) =>
      typeof selector === "function" ? selector(accountState) : accountState
    );

    mockCurrentAccountId.mockReturnValue("test-account-id");
  });

  it("renders operations feature with task data", () => {
    render(<OperationsFeature />);

    // tasks tab 默认激活，真实渲染任务内容
    expect(screen.getByText("测试任务")).toBeInTheDocument();
    // tab 标签真实渲染
    expect(screen.getByText("跟进任务")).toBeInTheDocument();
    expect(screen.getByText("复核记录")).toBeInTheDocument();
  });

  it("loads operations data on mount", () => {
    render(<OperationsFeature />);

    expect(mockLoadOperationsData).toHaveBeenCalledWith("test-account-id");
  });

  it("shows empty state when no tasks", () => {
    (useOperationsStore as any).mockReturnValue({
      events: [],
      tasks: [],
      decisionReviews: [],
      llmUsage: null,
      opsTab: "tasks",
      dataAccountId: "test-account-id",
      setOpsTab: vi.fn(),
      loadOperationsData: mockLoadOperationsData,
      reviewTaskNow: mockReviewTaskNow,
      cancelTask: mockCancelTask,
    });

    render(<OperationsFeature />);

    expect(screen.getByText("暂无跟进任务")).toBeInTheDocument();
  });

  it("跟进任务行点击「取消」把冻结任务实体交给 Store", async () => {
    render(<OperationsFeature />);
    const { fireEvent, waitFor } = await import("@testing-library/react");
    fireEvent.click(screen.getByText("取消"));
    await waitFor(() =>
      expect(mockCancelTask).toHaveBeenCalledWith(
        expect.objectContaining({ id: "1", accountId: "test-account-id" }),
        "test-account-id",
      ),
    );
  });

  it("跟进任务行点击「立即复核」把冻结任务实体交给 Store", async () => {
    render(<OperationsFeature />);
    const { fireEvent, waitFor } = await import("@testing-library/react");
    fireEvent.click(screen.getByText("立即复核"));
    await waitFor(() =>
      expect(mockReviewTaskNow).toHaveBeenCalledWith(
        expect.objectContaining({ id: "1", accountId: "test-account-id" }),
        "test-account-id",
      ),
    );
  });
  it("F15: loading 时显加载态而非空态", () => {
    (useOperationsStore as any).mockReturnValue({
      events: [],
      tasks: [],
      decisionReviews: [],
      llmUsage: null,
      agentRuns: [],
      loading: true,
      opsTab: "tasks",
      dataAccountId: "test-account-id",
      setOpsTab: vi.fn(),
      loadOperationsData: mockLoadOperationsData,
      reviewTaskNow: mockReviewTaskNow,
      cancelTask: mockCancelTask,
    });

    render(<OperationsFeature />);

    expect(screen.getByText("加载中…")).toBeInTheDocument();
    expect(screen.queryByText("暂无跟进任务")).not.toBeInTheDocument();
  });

  it("C10: 事件含结构化 detail 时渲染可展开明细", () => {
    (useOperationsStore as any).mockReturnValue({
      events: [
        {
          id: "e1",
          kind: "ptier_escalated",
          status: "ok",
          summary: "升档为完整知识档",
          detail: { runId: "run-9", knowledgeCoverage: 0.42 },
          createdAt: "2026-06-28T00:00:00Z",
        },
      ],
      tasks: [],
      decisionReviews: [],
      llmUsage: null,
      agentRuns: [],
      opsTab: "events",
      dataAccountId: "test-account-id",
      setOpsTab: vi.fn(),
      loadOperationsData: mockLoadOperationsData,
      reviewTaskNow: mockReviewTaskNow,
      cancelTask: mockCancelTask,
    });

    const { container } = render(<OperationsFeature />);

    expect(screen.getByText("结构化明细")).toBeInTheDocument();
    // detail 经 <pre>JSON.stringify</pre> 渲染，run_id/coverage 在文本中可读
    const pre = container.querySelector("pre");
    expect(pre?.textContent).toContain("run-9");
    expect(pre?.textContent).toContain("knowledgeCoverage");
  });

  it("C10: 事件无 detail 时不渲染明细块(向后兼容)", () => {
    (useOperationsStore as any).mockReturnValue({
      events: [
        {
          id: "e2",
          kind: "follow_up",
          status: "ok",
          summary: "跟进",
          createdAt: "2026-06-28T00:00:00Z",
        },
      ],
      tasks: [],
      decisionReviews: [],
      llmUsage: null,
      agentRuns: [],
      opsTab: "events",
      dataAccountId: "test-account-id",
      setOpsTab: vi.fn(),
      loadOperationsData: mockLoadOperationsData,
      reviewTaskNow: mockReviewTaskNow,
      cancelTask: mockCancelTask,
    });

    render(<OperationsFeature />);

    expect(screen.queryByText("结构化明细")).not.toBeInTheDocument();
  });
});

describe("operationsStore loading 生命周期(F15)", () => {
  it("F15: loadOperationsData 起置 loading=true,完成后 loading=false", async () => {
    const actual = await vi.importActual<typeof import("../../../stores/operationsStore")>(
      "../../../stores/operationsStore",
    );
    const store = actual.useOperationsStore;
    (useAccountStore as any).getState = () => ({
      currentAccountId: () => "acc",
    });
    // 受控 promise：让 5 个并行 api.get 的完成时机由测试掌控
    const resolvers: ((v: any) => void)[] = [];
    (api.get as any).mockImplementation(
      () => new Promise((res) => resolvers.push(res)),
    );

    const p = store.getState().loadOperationsData("acc");
    // 微任务前：loading 已置 true
    expect(store.getState().loading).toBe(true);

    resolvers.forEach((r) => r({ items: [] }));
    await p;
    // 完成后：finally 复位 loading=false
    expect(store.getState().loading).toBe(false);
  });
});

describe("formatScores 动态遍历(E13)", () => {
  it("渲染 boundaryPrivacySafety 隐私维度(不再被白名单丢弃)", () => {
    const out = formatScores({ humanLike: 8, boundaryPrivacySafety: 7 });
    expect(out).toContain("拟人度:8");
    expect(out).toContain("隐私边界:7");
  });
  it("未知 key 回落原始 key 名,不吞", () => {
    const out = formatScores({ someNewDimension: 5 });
    expect(out).toContain("someNewDimension:5");
  });
  it("空 scores 回落 -", () => {
    expect(formatScores({})).toBe("-");
  });
});

describe("决策复盘 reviews tab 拦截四分支(C8)", () => {
  const mockLoadOperationsData = vi.fn();
  const mockCurrentAccountId = vi.fn();

  function mountReviewsTab(decisionReviews: any[]) {
    vi.clearAllMocks();
    ((globalThis as any).fetch as any) = vi.fn().mockResolvedValue({
      ok: true,
      json: () => Promise.resolve({ items: [] }),
    });
    (useOperationsStore as any).mockReturnValue({
      events: [],
      tasks: [],
      decisionReviews,
      llmUsage: null,
      opsTab: "reviews",
      dataAccountId: "test-account-id",
      setOpsTab: vi.fn(),
      loadOperationsData: mockLoadOperationsData,
      reviewTaskNow: vi.fn(),
      cancelTask: vi.fn(),
    });
    const accountState = {
      accounts: [
        { id: "test-account-id", accountId: "test-account-id", alias: "测试", displayName: "测试", online: true },
      ],
      selectedAccountId: "test-account-id",
      currentAccountId: mockCurrentAccountId,
    };
    (useAccountStore as any).mockImplementation((selector?: any) =>
      typeof selector === "function" ? selector(accountState) : accountState
    );
    mockCurrentAccountId.mockReturnValue("test-account-id");
    render(<OperationsFeature />);
  }

  it("finalReviewStatus 存在时显具体分支标签而非裸「拦截」", () => {
    mountReviewsTab([
      {
        id: "r1",
        approved: false,
        finalReviewStatus: "blocked_unverified_product_claim",
        scores: {},
        risks: [],
        status: "blocked",
      },
    ]);
    expect(screen.getByText("未验证产品声明拦截")).toBeInTheDocument();
    expect(screen.queryByText("拦截", { exact: true })).not.toBeInTheDocument();
  });

  it("仅 holdCategory 存在时按 HOLD_CATEGORY_LABELS 显标签", () => {
    mountReviewsTab([
      {
        id: "r2",
        approved: false,
        holdCategory: "blocked_by_safety_guard",
        scores: {},
        risks: [],
        status: "blocked",
      },
    ]);
    expect(screen.getByText("安全门拦截")).toBeInTheDocument();
  });

  it("两字段都缺失时回落二元「拦截」(向后兼容)", () => {
    mountReviewsTab([
      {
        id: "r3",
        approved: false,
        scores: {},
        risks: [],
        status: "blocked",
      },
    ]);
    expect(screen.getByText("拦截", { exact: true })).toBeInTheDocument();
  });

  it("approved 时显「通过」不受新字段影响", () => {
    mountReviewsTab([
      {
        id: "r4",
        approved: true,
        finalReviewStatus: "approved",
        scores: {},
        risks: [],
        status: "sent",
      },
    ]);
    expect(screen.getByText("通过")).toBeInTheDocument();
  });
});

describe("运行日志 runs tab + tier 遥测(C6+C9)", () => {
  const mockLoadOperationsData = vi.fn();
  const mockCurrentAccountId = vi.fn();

  function mountRunsTab(agentRuns: any[]) {
    vi.clearAllMocks();
    ((globalThis as any).fetch as any) = vi.fn().mockResolvedValue({
      ok: true,
      json: () => Promise.resolve({ items: [] }),
    });
    (useOperationsStore as any).mockReturnValue({
      events: [],
      tasks: [],
      decisionReviews: [],
      llmUsage: null,
      agentRuns,
      opsTab: "runs",
      dataAccountId: "test-account-id",
      setOpsTab: vi.fn(),
      loadOperationsData: mockLoadOperationsData,
      loadAgentRuns: vi.fn(),
      reviewTaskNow: vi.fn(),
      cancelTask: vi.fn(),
    });
    const accountState = {
      accounts: [
        { id: "test-account-id", accountId: "test-account-id", alias: "测试", displayName: "测试", online: true },
      ],
      selectedAccountId: "test-account-id",
      currentAccountId: mockCurrentAccountId,
    };
    (useAccountStore as any).mockImplementation((selector?: any) =>
      typeof selector === "function" ? selector(accountState) : accountState
    );
    mockCurrentAccountId.mockReturnValue("test-account-id");
    render(<OperationsFeature />);
  }

  it("空运行日志显示 EmptyState", () => {
    mountRunsTab([]);
    expect(screen.getByText("暂无运行日志")).toBeInTheDocument();
  });

  it("摘要行渲染 runId/triggerKind/status + 档位遥测来自 decision", () => {
    mountRunsTab([
      {
        id: "r1",
        runId: "run-1",
        status: "sent",
        triggerKind: "inbound",
        decision: { sufficiency: "need_more_context", missingTier: "full" },
        gatewayResult: { status: "sent" },
      },
    ]);
    expect(screen.getByText("run-1")).toBeInTheDocument();
    expect(screen.getByText("inbound")).toBeInTheDocument();
    // 档位遥测从 decision.missingTier 派生(Full)且标记需升档
    expect(screen.getByText(/需完整知识档.*需升档/)).toBeInTheDocument();
  });

  it("展开后显式列出 tier 遥测三字段(档位/充分性/是否升档)", async () => {
    mountRunsTab([
      {
        id: "r2",
        runId: "run-2",
        status: "sent",
        triggerKind: "inbound",
        decision: { sufficiency: "enough", missingTier: "none" },
      },
    ]);
    const { fireEvent } = await import("@testing-library/react");
    fireEvent.click(screen.getByText("展开"));
    expect(screen.getByText(/充分性:信息充分/)).toBeInTheDocument();
    expect(screen.getByText(/是否升档:否/)).toBeInTheDocument();
    // decision 阶段通用 key-value 渲染暴露原始键
    expect(screen.getByText("missingTier")).toBeInTheDocument();
  });

  // ── 运行日志表格布局回归（阶段明细溢出修复）────────────────────────
  // 缺陷：thead 只有 5 个 th，而摘要行有 6 个 td（缺「操作」列表头），
  // 导致列宽分配错位、末列失去表头约束。
  it("runs 表头列数与摘要行单元格数一致", () => {
    mountRunsTab([
      { id: "r3", runId: "run-3", status: "sent", triggerKind: "inbound" },
    ]);
    const table = document.querySelector("table");
    expect(table).not.toBeNull();
    const thCount = table!.querySelectorAll("thead th").length;
    const tdCount = table!.querySelectorAll("tbody tr:first-child > td").length;
    expect(thCount).toBe(tdCount);
  });

  // 缺陷：阶段区块误用 .tHead（display:flex 横向容器，为事件时间线的
  // <strong>+<span> 设计），内含 width:100% 的嵌套表格 → flex 项
  // min-width:auto 使表格按内容撑开、标签被挤成逐字竖排、整体横向溢出。
  it("展开后的阶段区块不使用事件时间线的 flex 容器类", async () => {
    mountRunsTab([
      {
        id: "r4",
        runId: "run-4",
        status: "sent",
        triggerKind: "inbound",
        planner: { riskLevel: "medium", reason: "测试" },
      },
    ]);
    const { fireEvent } = await import("@testing-library/react");
    fireEvent.click(screen.getByText("展开"));
    // 找到包含嵌套 table 的阶段区块容器
    const stageBlock = screen.getByText("规划").closest("div");
    expect(stageBlock).not.toBeNull();
    expect(stageBlock!.querySelector("table")).not.toBeNull();
    // 不得复用 tHead（那是 flex 横向容器，会撑破布局）
    expect(stageBlock!.className).not.toMatch(/tHead/);
    // 必须挂上专用的阶段区块类
    expect(stageBlock!.className).toMatch(/stageBlock/);
  });

  // 缺陷：嵌套表格无固定布局与断词，长 JSON 值（toolTrace / chunkId 数组）
  // 会撑破列宽。修复后表格须挂 .stageTable（table-layout:fixed + word-break）。
  it("阶段明细表格挂 stageTable 类（固定布局 + 断词）", async () => {
    mountRunsTab([
      {
        id: "r5",
        runId: "run-5",
        status: "sent",
        triggerKind: "inbound",
        knowledgeRoute: {
          selectedChunkIds: ["68a1f2c4d5e6a7b8c9d0e1f2", "68a1f2c4d5e6a7b8c9d0e1f3"],
        },
      },
    ]);
    const { fireEvent } = await import("@testing-library/react");
    fireEvent.click(screen.getByText("展开"));
    const stageBlock = screen.getByText("知识路由").closest("div");
    const nested = stageBlock!.querySelector("table");
    expect(nested).not.toBeNull();
    expect(nested!.className).toMatch(/stageTable/);
  });
});

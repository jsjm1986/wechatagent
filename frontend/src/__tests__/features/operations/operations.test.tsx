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
      tasks: [{ id: "1", content: "测试任务", status: "pending" }],
      decisionReviews: [],
      llmUsage: {
        summary: {
          totalCalls: 0,
          totalTokens: 0,
          promptCacheHitTokens: 0,
          promptCacheMissTokens: 0,
          promptCacheHitRate: 0,
        },
        items: [],
      },
      opsTab: "tasks",
      setOpsTab: vi.fn(),
      loadOperationsData: mockLoadOperationsData,
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
    expect(screen.getByText("Review 记录")).toBeInTheDocument();
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
      setOpsTab: vi.fn(),
      loadOperationsData: mockLoadOperationsData,
    });

    render(<OperationsFeature />);

    expect(screen.getByText("暂无跟进任务")).toBeInTheDocument();
  });

  it("跟进任务行点击「取消」调用 cancel 端点", async () => {
    render(<OperationsFeature />);
    const { fireEvent, waitFor } = await import("@testing-library/react");
    fireEvent.click(screen.getByText("取消"));
    await waitFor(() =>
      expect(api.post).toHaveBeenCalledWith("/api/agent-tasks/1/cancel"),
    );
  });

  it("跟进任务行点击「立即复核」调用 review-now 端点", async () => {
    render(<OperationsFeature />);
    const { fireEvent, waitFor } = await import("@testing-library/react");
    fireEvent.click(screen.getByText("立即复核"));
    await waitFor(() =>
      expect(api.post).toHaveBeenCalledWith("/api/agent-tasks/1/review-now"),
    );
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

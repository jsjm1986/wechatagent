import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import OperationsFeature from "../../../features/operations";
import { useOperationsStore } from "../../../stores/operationsStore";
import { useAccountStore } from "../../../stores/accountStore";

// Mock stores
vi.mock("../../../stores/operationsStore");
vi.mock("../../../stores/accountStore");

// Mock OperationsView component
vi.mock("../../../App", () => ({
  OperationsView: ({ tasks, opsTab }: any) => (
    <div>
      <div data-testid="ops-tab">{opsTab}</div>
      {tasks.length > 0 && (
        <div data-testid="tasks-content">
          {tasks.map((task: any) => (
            <div key={task.id} data-testid={`task-${task.id}`}>
              {task.content}
            </div>
          ))}
        </div>
      )}
      {tasks.length === 0 && <div data-testid="empty-tasks">暂无跟进任务</div>}
    </div>
  ),
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

    (useAccountStore as any).mockReturnValue({
      currentAccountId: mockCurrentAccountId,
    });

    mockCurrentAccountId.mockReturnValue("test-account-id");
  });

  it("renders operations feature with task data", () => {
    render(<OperationsFeature />);

    expect(screen.getByTestId("ops-tab")).toHaveTextContent("tasks");
    expect(screen.getByTestId("tasks-content")).toBeInTheDocument();
    expect(screen.getByTestId("task-1")).toHaveTextContent("测试任务");
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

    expect(screen.getByTestId("empty-tasks")).toHaveTextContent("暂无跟进任务");
  });
});
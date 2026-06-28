import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TaskRail } from "../../../features/knowledge/today";

function mockFetch(handler: (url: string, init?: RequestInit) => unknown) {
  globalThis.fetch = vi.fn(async (url: unknown, init?: RequestInit) => {
    const body = handler(String(url), init);
    return {
      ok: true,
      status: 200,
      async json() { return body; },
      async text() { return JSON.stringify(body); },
    } as unknown as Response;
  }) as typeof fetch;
}

describe("TaskRail 任务总览列表", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("挂载时拉取任务列表并渲染列表项", async () => {
    mockFetch((url) => {
      if (url.includes("/knowledge/chat/tasks/")) {
        return { taskId: "T1", sessionId: "S1", status: "running", totalSteps: 3, completedSteps: [1], cards: [] };
      }
      if (url.includes("/knowledge/chat/tasks")) {
        return { items: [
          { taskId: "T1", sessionId: "S1", status: "running", totalSteps: 3, completedStepCount: 1, createdAt: "2026-06-28" },
          { taskId: "T2", sessionId: "S2", status: "completed", totalSteps: 2, completedStepCount: 2, createdAt: "2026-06-27" },
        ] };
      }
      return {};
    });
    render(<TaskRail />);
    await waitFor(() => expect(screen.getByText("S1")).toBeInTheDocument());
    expect(screen.getByText("S2")).toBeInTheDocument();
    // 进度文本用 completedStepCount 数字渲染（防止被误改成对数组取 .length）。
    expect(screen.getByText("1/3")).toBeInTheDocument();
    expect(screen.getByText("2/2")).toBeInTheDocument();
  });

  it("点选列表项触发 loadTask 拉详情", async () => {
    const user = userEvent.setup();
    const calls: string[] = [];
    mockFetch((url) => {
      calls.push(url);
      if (url.includes("/knowledge/chat/tasks/")) {
        return { taskId: "T1", sessionId: "S1", status: "running", totalSteps: 3, completedSteps: [1], cards: [] };
      }
      if (url.includes("/knowledge/chat/tasks")) {
        return { items: [{ taskId: "T1", sessionId: "S1", status: "running", totalSteps: 3, completedStepCount: 1, createdAt: "2026-06-28" }] };
      }
      return {};
    });
    render(<TaskRail />);
    await waitFor(() => expect(screen.getByText("S1")).toBeInTheDocument());
    await user.click(screen.getByText("S1"));
    await waitFor(() =>
      expect(calls.some((u) => u.includes("/knowledge/chat/tasks/T1"))).toBe(true)
    );
  });
});

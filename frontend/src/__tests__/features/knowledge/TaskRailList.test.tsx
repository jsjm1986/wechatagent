import { afterEach, describe, it, expect, vi, beforeEach } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TaskRail } from "../../../features/knowledge/today";
import { ToastProvider } from "../../../components/ui/Toast";

class FakeTaskEventSource {
  static instances: FakeTaskEventSource[] = [];
  listeners: Record<string, Array<(event: { data?: string }) => void>> = {};
  closed = false;

  constructor(public url: string) {
    FakeTaskEventSource.instances.push(this);
  }

  addEventListener(type: string, callback: (event: { data?: string }) => void) {
    (this.listeners[type] ||= []).push(callback);
  }

  close() {
    this.closed = true;
  }

  emit(type: string, data?: string) {
    for (const callback of this.listeners[type] ?? []) callback({ data });
  }
}

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
  beforeEach(() => {
    vi.restoreAllMocks();
    FakeTaskEventSource.instances = [];
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

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
    render(
      <ToastProvider>
        <TaskRail />
      </ToastProvider>
    );
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
    render(
      <ToastProvider>
        <TaskRail />
      </ToastProvider>
    );
    await waitFor(() => expect(screen.getByText("S1")).toBeInTheDocument());
    await user.click(screen.getByText("S1"));
    await waitFor(() =>
      expect(calls.some((u) => u.includes("/knowledge/chat/tasks/T1"))).toBe(true)
    );
  });

  it("收到 turn 后回读权威任务详情并更新主进度", async () => {
    vi.stubGlobal("EventSource", FakeTaskEventSource as unknown as typeof EventSource);
    let detailReads = 0;
    mockFetch((url) => {
      if (url.includes("/knowledge/chat/tasks/T1")) {
        detailReads += 1;
        return {
          taskId: "T1",
          sessionId: "S1",
          status: "running",
          totalSteps: 2,
          completedSteps: detailReads >= 2 ? [1] : [],
          cards: [],
        };
      }
      if (url.includes("/knowledge/chat/tasks")) {
        return {
          items: [{
            taskId: "T1",
            sessionId: "S1",
            status: "running",
            totalSteps: 2,
            completedStepCount: 0,
          }],
        };
      }
      return {};
    });

    render(
      <ToastProvider>
        <TaskRail />
      </ToastProvider>,
    );
    await waitFor(() => expect(screen.getByText("S1")).toBeInTheDocument());
    await act(async () => {
      fireEvent.click(screen.getByText("S1"));
      await Promise.resolve();
    });
    expect(FakeTaskEventSource.instances).toHaveLength(1);

    await act(async () => {
      FakeTaskEventSource.instances[0].emit("turn", "1");
      await Promise.resolve();
    });

    await waitFor(() => expect(detailReads).toBe(2));
    expect(screen.getByText("1/2 步")).toBeInTheDocument();
    expect(screen.getByText("第 1 步")).toBeInTheDocument();
  });

  it("SSE 连续失败后有限轮询收敛终态并停止", async () => {
    vi.useFakeTimers();
    vi.stubGlobal("EventSource", FakeTaskEventSource as unknown as typeof EventSource);
    let detailReads = 0;
    mockFetch((url) => {
      if (url.includes("/knowledge/chat/tasks/T1")) {
        detailReads += 1;
        const completed = detailReads >= 3;
        return {
          taskId: "T1",
          sessionId: "S1",
          status: completed ? "completed" : "running",
          totalSteps: 1,
          completedSteps: completed ? [1] : [],
          cards: [],
          finishedAt: completed ? "2026-07-19T00:00:00Z" : null,
        };
      }
      if (url.includes("/knowledge/chat/tasks")) {
        return {
          items: [
            {
              taskId: "T1",
              sessionId: "S1",
              status: "running",
              totalSteps: 1,
              completedStepCount: 0,
            },
          ],
        };
      }
      return {};
    });

    render(
      <ToastProvider>
        <TaskRail />
      </ToastProvider>,
    );
    await act(async () => { await Promise.resolve(); });
    await act(async () => {
      fireEvent.click(screen.getByText("S1"));
      await Promise.resolve();
    });
    expect(FakeTaskEventSource.instances).toHaveLength(1);

    const delays = [1_000, 2_000, 4_000, 8_000, 16_000, 30_000];
    for (const delay of delays) {
      await act(async () => {
        FakeTaskEventSource.instances.at(-1)!.emit("error");
        vi.advanceTimersByTime(delay);
        await Promise.resolve();
      });
    }
    await act(async () => {
      FakeTaskEventSource.instances.at(-1)!.emit("error");
      await Promise.resolve();
    });

    expect(detailReads).toBe(2);
    expect(screen.getByText(/正在通过任务接口核对最新状态/)).toBeInTheDocument();

    await act(async () => { vi.advanceTimersByTime(5_000); });
    expect(detailReads).toBe(3);
    expect(screen.getByText("已完成")).toBeInTheDocument();
    expect(screen.queryByText(/正在通过任务接口核对最新状态/)).not.toBeInTheDocument();

    await act(async () => { vi.advanceTimersByTime(60_000); });
    expect(detailReads).toBe(3);
  });

  it("无 EventSource 时有限轮询达到上限后提示手工拉取", async () => {
    vi.useFakeTimers();
    vi.stubGlobal("EventSource", undefined);
    let detailReads = 0;
    mockFetch((url) => {
      if (url.includes("/knowledge/chat/tasks/T1")) {
        detailReads += 1;
        return {
          taskId: "T1",
          sessionId: "S1",
          status: "running",
          totalSteps: 1,
          completedSteps: [],
          cards: [],
        };
      }
      if (url.includes("/knowledge/chat/tasks")) {
        return {
          items: [{
            taskId: "T1",
            sessionId: "S1",
            status: "running",
            totalSteps: 1,
            completedStepCount: 0,
          }],
        };
      }
      return {};
    });

    render(
      <ToastProvider>
        <TaskRail />
      </ToastProvider>,
    );
    await act(async () => { await Promise.resolve(); });
    await act(async () => {
      fireEvent.click(screen.getByText("S1"));
      await Promise.resolve();
    });

    // 1 次手工详情 + fallback 立即核对 1 次，之后每 5 秒最多再核对 11 次。
    expect(detailReads).toBe(2);
    for (let i = 0; i < 11; i++) {
      await act(async () => {
        vi.advanceTimersByTime(5_000);
        await Promise.resolve();
      });
    }
    expect(detailReads).toBe(13);
    expect(screen.getByText(/自动核对均已停止，请点击“拉取”/)).toBeInTheDocument();

    await act(async () => { vi.advanceTimersByTime(60_000); });
    expect(detailReads).toBe(13);
  });
});

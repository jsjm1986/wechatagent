import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AskView } from "../../../features/knowledge/explore";

// F14：explore 的「租户（可选）」输入框是误导性死控件 —— 后端 sources_meta.rs
// 无条件用 admin.current_workspace，请求里的 workspaceId 被忽略。切租户的正确路径
// 是顶部 workspace 切换器走 /api/auth/workspace。本测试守住「该输入框已被移除」。

const realFetch = globalThis.fetch;

describe("AskView — F14 移除误导性租户输入框", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    globalThis.fetch = vi.fn(async () => {
      const body = {
        answer: "",
        citedChunkIds: [],
        sourceQuotes: [],
        toolTrace: [],
        roundsUsed: 0,
        truncated: false,
        tookMs: 0,
      };
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
  });
  afterEach(() => {
    globalThis.fetch = realFetch;
  });

  it("不再渲染租户输入框（placeholder=default）与「租户（可选）」文案", () => {
    render(<AskView />);
    // 死控件用 placeholder "default"，移除后 query 不到。
    expect(screen.queryByPlaceholderText("default")).toBeNull();
    // 标签文案也应消失。
    expect(screen.queryByText(/租户（可选）/)).toBeNull();
  });
});

// F17：explore 一次性流 error handler 的 stale closure 误抑制错误横幅。
// 上一轮拿到 result 后再次提交，resetForSubmit 的 setResult(null) 是异步 state，
// error handler 同步触发时闭包捕获的仍是旧非空 result → !result 为 false → 新查询
// 失败时错误横幅被误抑制。本测试守住「先成功再失败」时第二次失败的横幅必须出现。

class FakeESForF17 {
  static instances: FakeESForF17[] = [];
  url: string;
  listeners: Record<string, ((ev: unknown) => void)[]> = {};
  closed = false;
  constructor(url: string) {
    this.url = url;
    FakeESForF17.instances.push(this);
  }
  addEventListener(t: string, cb: (ev: unknown) => void) {
    (this.listeners[t] ||= []).push(cb);
  }
  close() {
    this.closed = true;
  }
  emit(t: string, data?: unknown) {
    (this.listeners[t] || []).forEach((cb) => cb({ data: JSON.stringify(data) }));
  }
}

describe("AskView — F17 stale closure 误抑制错误横幅", () => {
  beforeEach(() => {
    FakeESForF17.instances = [];
    vi.stubGlobal("EventSource", FakeESForF17 as unknown as typeof EventSource);
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("F17: 上一轮有结果后再次提交失败,错误横幅不被旧 result 抑制", async () => {
    render(<AskView />);
    const textarea = screen.getByPlaceholderText(/向知识库提一个问题/);

    // 第一次提交 → 成功拿到 answer（result 非空）
    fireEvent.change(textarea, { target: { value: "第一个问题" } });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /提问/ }));
    });
    expect(FakeESForF17.instances).toHaveLength(1);
    await act(async () => {
      FakeESForF17.instances[0].emit("answer", {
        answer: "这是第一轮的答案",
        citedChunkIds: [],
        sourceQuotes: [],
        toolTrace: [],
        roundsUsed: 1,
        truncated: false,
      });
      FakeESForF17.instances[0].emit("close");
    });
    expect(screen.getByText("这是第一轮的答案")).toBeTruthy();

    // 第二次提交 → 直接 error（无 answer）。stale closure 下旧 result 仍非空会抑制横幅。
    fireEvent.change(textarea, { target: { value: "第二个问题" } });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /提问/ }));
    });
    expect(FakeESForF17.instances).toHaveLength(2);
    await act(async () => {
      FakeESForF17.instances[1].emit("error");
    });

    // 修复后：resetForSubmit 已同步清空 resultRef → 错误横幅必须出现。
    expect(screen.getByText(/流式连接错误/)).toBeTruthy();
  });

  it("业务 failed 终态展示服务端通用文案并结束 pending", async () => {
    render(<AskView />);
    const textarea = screen.getByPlaceholderText(/向知识库提一个问题/);
    fireEvent.change(textarea, { target: { value: "触发失败" } });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /提问/ }));
    });

    await act(async () => {
      FakeESForF17.instances[0].emit("failed", {
        code: "knowledge_agent_failed",
        message: "知识问答暂时失败，请稍后重试。",
      });
    });

    expect(screen.getByText("知识问答暂时失败，请稍后重试。")).toBeTruthy();
    expect(screen.getByRole("button", { name: "提问" })).toBeTruthy();
    expect(FakeESForF17.instances[0].closed).toBe(true);
  });
});

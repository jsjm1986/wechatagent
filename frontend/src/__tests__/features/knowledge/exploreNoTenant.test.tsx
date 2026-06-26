import { render, screen } from "@testing-library/react";
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

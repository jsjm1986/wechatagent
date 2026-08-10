import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { DigestCanvas } from "../../../features/knowledge/today";

// 线上事故复现用例。
//
// 生产以纯 HTTP + IP 提供服务 → 非安全上下文 → `crypto.randomUUID` 是 undefined，
// 「批量派工」在 fetch 之前就抛 `crypto.randomUUID is not a function`，请求从未发出。
// 而 jsdom 跑 Node webcrypto、dev server 跑 localhost，两边 randomUUID 都健在，
// 所以既有的批量派工用例全绿也挡不住。这里**显式摘掉** randomUUID 复现宿主。

const realFetch = globalThis.fetch;
const realCrypto = globalThis.crypto;

function jsonResponse(body: unknown): Response {
  return {
    ok: true,
    status: 200,
    json: async () => body,
    text: async () => JSON.stringify(body),
  } as Response;
}

const REPORT = {
  reportId: "fedcba987654321001234567",
  reportHash: "report-hash",
  workspaceId: "ws-a",
  accountId: "account-a",
  reportDate: "2026-07-27",
  status: "ok",
  currentGeneration: 7,
  cards: [
    {
      cardId: "0123456789abcdef01234567",
      cardHash: "card-hash",
      kind: "chunk_missing_field",
      title: "Bound card",
      summary: "Authoritative summary",
      severity: "warn",
      suggestedAction: "fix_chunk",
    },
  ],
  dismissedCardIds: [],
};

describe("DigestCanvas 在非安全上下文（生产形态）", () => {
  afterEach(() => {
    globalThis.fetch = realFetch;
    Object.defineProperty(globalThis, "crypto", {
      configurable: true,
      writable: true,
      value: realCrypto,
    });
    vi.restoreAllMocks();
  });

  it("crypto.randomUUID 缺失时，批量派工仍能把请求发出去", async () => {
    // 只留 getRandomValues：这正是 HTTP + IP 下浏览器暴露的能力集。
    Object.defineProperty(globalThis, "crypto", {
      configurable: true,
      writable: true,
      value: { getRandomValues: realCrypto.getRandomValues.bind(realCrypto) },
    });

    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.includes("/api/knowledge/digest/today")) return jsonResponse(REPORT);
      if (url === "/api/knowledge/chat/tasks" && init?.method === "POST") {
        return jsonResponse({ taskId: "task-a", status: "pending" });
      }
      return jsonResponse({ items: [] });
    });
    globalThis.fetch = fetchMock as typeof fetch;

    const user = userEvent.setup();
    render(<DigestCanvas />);
    await user.click(await screen.findByRole("checkbox", { name: "选择卡片 Bound card" }));
    await user.click(screen.getByRole("button", { name: "批量派工（1）" }));

    await waitFor(() => {
      const call = fetchMock.mock.calls.find(
        ([url, init]) => String(url) === "/api/knowledge/chat/tasks" && init?.method === "POST",
      );
      expect(call, "派工请求必须真的发出（此前在 fetch 前就抛异常）").toBeTruthy();
      const body = JSON.parse(String(call?.[1]?.body));
      expect(typeof body.sessionId).toBe("string");
      expect(body.sessionId.length).toBeGreaterThan(0);
    });

    // 不得出现错误横幅：请求成功，不该有任何报错上屏。
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("本地异常走「客户端错误」横幅，重试按钮不写「AI 重试」", async () => {
    // 让派工在 fetch 阶段抛一个本地异常（模拟浏览器端故障），
    // 验证它不再被冒充成 LLM 上游故障。
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.includes("/api/knowledge/digest/today")) return jsonResponse(REPORT);
      if (url === "/api/knowledge/chat/tasks" && init?.method === "POST") {
        throw new TypeError("crypto.randomUUID is not a function");
      }
      return jsonResponse({ items: [] });
    });
    globalThis.fetch = fetchMock as typeof fetch;

    const user = userEvent.setup();
    render(<DigestCanvas />);
    await user.click(await screen.findByRole("checkbox", { name: "选择卡片 Bound card" }));
    await user.click(screen.getByRole("button", { name: "批量派工（1）" }));

    const banner = await screen.findByRole("alert");
    expect(banner).toHaveTextContent("客户端错误");
    expect(banner).not.toHaveTextContent("未知错误");
    // onRetry 绑的是 load()（重新拉取摘要），不是重发派工 —— 文案必须如实。
    expect(screen.getByRole("button", { name: "重新加载" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "AI 重试" })).toBeNull();
  });
});

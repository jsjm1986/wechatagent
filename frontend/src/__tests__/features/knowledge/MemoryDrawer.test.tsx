import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ConfirmProvider } from "../../../components/ui/ConfirmDialog";
import { ToastProvider } from "../../../components/ui/Toast";
import { MemoryDrawer } from "../../../features/knowledge/atlas";

const realFetch = globalThis.fetch;

afterEach(() => {
  globalThis.fetch = realFetch;
  vi.restoreAllMocks();
});

function response(body: unknown, ok = true): Response {
  return {
    ok,
    status: ok ? 200 : 500,
    json: async () => body,
    text: async () => JSON.stringify(body),
  } as Response;
}

describe("MemoryDrawer operator-memory revocation", () => {
  it("freezes the selected scope, revokes it, and removes only that active row", async () => {
    const requests: Array<{ url: string; method: string; body?: string }> = [];
    globalThis.fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      const method = init?.method ?? "GET";
      requests.push({ url, method, body: init?.body as string | undefined });
      if (method === "POST") {
        return response({ id: "memory-a", alreadyRevoked: false });
      }
      return response({
        items: [
          {
            id: "memory-a",
            workspaceId: "ws-a",
            accountId: "account-a",
            operatorId: "operator-a",
            kind: "preference",
            content: "回复保持简洁",
          },
          {
            id: "memory-b",
            workspaceId: "ws-a",
            accountId: "account-a",
            operatorId: "operator-a",
            kind: "context",
            content: "周五复盘",
          },
        ],
      });
    }) as typeof fetch;

    const user = userEvent.setup();
    render(
      <ToastProvider>
        <ConfirmProvider>
          <MemoryDrawer />
        </ConfirmProvider>
      </ToastProvider>,
    );

    await screen.findByText("回复保持简洁");
    const revokeButtons = screen.getAllByRole("button", { name: "撤销" });
    await user.click(revokeButtons[0]);
    expect(screen.getByText(/AI 后续起草将不再参考/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "确认撤销" }));

    await waitFor(() => expect(screen.queryByText("回复保持简洁")).not.toBeInTheDocument());
    expect(screen.getByText("周五复盘")).toBeInTheDocument();
    const post = requests.find((request) => request.method === "POST");
    expect(post?.url).toBe("/api/knowledge/operator-memory/memory-a/revoke");
    expect(JSON.parse(post?.body ?? "{}")).toEqual({
      accountId: "account-a",
      operatorId: "operator-a",
      reason: "运营在记忆抽屉中撤销",
    });
  });

  it("keeps the row visible when revocation fails", async () => {
    globalThis.fetch = vi.fn(async (_input: RequestInfo | URL, init?: RequestInit) =>
      init?.method === "POST"
        ? response({ error: "operator memory not found" }, false)
        : response({
            items: [{
              id: "memory-a",
              workspaceId: "ws-a",
              accountId: "account-a",
              operatorId: "operator-a",
              kind: "preference",
              content: "回复保持简洁",
            }],
          }),
    ) as typeof fetch;

    const user = userEvent.setup();
    render(
      <ToastProvider>
        <ConfirmProvider>
          <MemoryDrawer />
        </ConfirmProvider>
      </ToastProvider>,
    );
    await screen.findByText("回复保持简洁");
    await user.click(screen.getByRole("button", { name: "撤销" }));
    await user.click(screen.getByRole("button", { name: "确认撤销" }));
    await waitFor(() => expect(screen.getByText("回复保持简洁")).toBeInTheDocument());
  });
});

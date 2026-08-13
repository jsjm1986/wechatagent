import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ConfirmProvider } from "../../../components/ui/ConfirmDialog";
import { ToastProvider } from "../../../components/ui/Toast";
import { FormDialogProvider } from "../../../components/ui/FormDialog";
import { ChunkInspectorPane } from "../../../features/knowledge/shared";

// B1：反向引用查询的参数名契约。
// 后端 ChunkReferrersQuery 是 camelCase 契约（主名 targetId，target_id 仅为历史别名）；
// 前端必须发 targetId，否则曾出现恒 400（缺陷 #14）。

const realFetch = globalThis.fetch;

const SOURCE_ID = "chunk-src";

function jsonResponse(body: unknown): Response {
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
}

function installFetch() {
  globalThis.fetch = vi.fn(async (url: unknown) => {
    const u = String(url);
    if (u.includes("/lock")) {
      return jsonResponse({
        lock: { owner_user_id: "u1", owner_username: "admin", expires_at: "" },
      });
    }
    if (u.includes("/referrers")) {
      return jsonResponse({
        items: [
          {
            chunkId: "chunk-ref",
            title: "引用方",
            wikiType: "methodology",
            status: "active",
            kind: "supports",
            note: null,
          },
        ],
      });
    }
    return jsonResponse({
      items: [{ id: SOURCE_ID, title: "源知识", relatedChunks: [] }],
    });
  }) as typeof fetch;
}

function renderInspector() {
  return render(
    <ConfirmProvider>
      <ToastProvider>
        <FormDialogProvider>
          <ChunkInspectorPane chunkId={SOURCE_ID} onClose={() => {}} onClear={() => {}} />
        </FormDialogProvider>
      </ToastProvider>
    </ConfirmProvider>,
  );
}

describe("ChunkReferrersList — 反向引用查询参数契约", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    installFetch();
  });
  afterEach(() => {
    globalThis.fetch = realFetch;
  });

  it("展开「被引用」后以 camelCase targetId 请求 referrers", async () => {
    const user = userEvent.setup();
    renderInspector();

    const head = await screen.findByRole("button", { name: /被引用/ });
    await user.click(head);

    await waitFor(() => {
      const calls = (globalThis.fetch as ReturnType<typeof vi.fn>).mock.calls
        .map((c) => String(c[0]))
        .filter((u) => u.includes("/referrers"));
      expect(calls).toHaveLength(1);
      expect(calls[0]).toContain(
        `/api/operation-knowledge/chunks/referrers?targetId=${SOURCE_ID}`,
      );
      expect(calls[0]).not.toContain("target_id=");
    });

    // 响应渲染正常（列表出现引用方标题）。
    await screen.findByText("引用方");
  });
});

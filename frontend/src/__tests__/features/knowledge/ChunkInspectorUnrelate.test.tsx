import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ConfirmProvider } from "../../../components/ui/ConfirmDialog";
import { ToastProvider } from "../../../components/ui/Toast";
import { FormDialogProvider } from "../../../components/ui/FormDialog";
import { ChunkInspectorPane } from "../../../features/knowledge/shared";
import { api } from "../../../lib/api";

// E5：related_chunks 列表项「解除关联」按钮。
// 断言点击解除 → confirm → api.delete 打到 DELETE /relate/<target_id>，
// 且 :id 是源 chunk.id，:target_id 是 related 项的 chunk_id。

const realFetch = globalThis.fetch;

const SOURCE_ID = "chunk-src";
const TARGET_ID = "chunk-tgt";

function installFetch() {
  // /chunks 返回源 chunk（带 relatedChunks）+ 目标 chunk（让关联非 dead）。
  // /lock 返回 self 锁占位。
  globalThis.fetch = vi.fn(async (url: unknown) => {
    const u = String(url);
    if (u.includes("/lock")) {
      const body = { lock: { owner_user_id: "u1", owner_username: "admin", expires_at: "" } };
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
    const body = {
      items: [
        {
          id: SOURCE_ID,
          title: "源知识",
          relatedChunks: [{ chunk_id: TARGET_ID, kind: "supports", note: null }],
        },
        { id: TARGET_ID, title: "目标知识", relatedChunks: [] },
      ],
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

describe("ChunkInspectorPane — E5 解除关联 unrelate", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    installFetch();
  });
  afterEach(() => {
    globalThis.fetch = realFetch;
  });

  it("unrelate button DELETEs relate/:target_id (源 id + target id)", async () => {
    const delSpy = vi
      .spyOn(api, "delete")
      .mockResolvedValue({ ok: true, removed: 1 } as never);
    const user = userEvent.setup();
    renderInspector();

    // 等关联项渲染出来。
    const unrelateBtn = await screen.findByRole("button", { name: "解除关联" });
    await user.click(unrelateBtn);

    // confirm 弹窗出现，点确认。
    const confirmBtn = await screen.findByText("确认解除");
    await user.click(confirmBtn);

    await waitFor(() => {
      expect(delSpy).toHaveBeenCalledWith(
        `/api/operation-knowledge/chunks/${SOURCE_ID}/relate/${TARGET_ID}`,
      );
    });
  });

  it("dead 关联项（target 不在活跃集合）解除按钮仍可点且 DELETE 正常打出", async () => {
    // 覆盖默认 fetch：源 chunk 关联指向不在 items 里的目标 → 该关联项为 dead。
    globalThis.fetch = vi.fn(async (url: unknown) => {
      const u = String(url);
      if (u.includes("/lock")) {
        const body = { lock: { owner_user_id: "u1", owner_username: "admin", expires_at: "" } };
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
      // 注意：items 里没有 TARGET_ID，故关联项 dead（跳转禁用、解除仍可用）。
      const body = {
        items: [
          {
            id: SOURCE_ID,
            title: "源知识",
            relatedChunks: [{ chunk_id: TARGET_ID, kind: "supports", note: null }],
          },
        ],
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

    const delSpy = vi
      .spyOn(api, "delete")
      .mockResolvedValue({ ok: true, removed: 1 } as never);
    const user = userEvent.setup();
    renderInspector();

    // dead 项的解除按钮仍可点（disabled 只看 unrelating 态，不看 dead）。
    const unrelateBtn = await screen.findByRole("button", { name: "解除关联" });
    expect(unrelateBtn).not.toBeDisabled();
    await user.click(unrelateBtn);

    const confirmBtn = await screen.findByText("确认解除");
    await user.click(confirmBtn);

    await waitFor(() => {
      expect(delSpy).toHaveBeenCalledWith(
        `/api/operation-knowledge/chunks/${SOURCE_ID}/relate/${TARGET_ID}`,
      );
    });
  });
});

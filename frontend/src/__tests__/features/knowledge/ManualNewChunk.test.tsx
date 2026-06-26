import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ConfirmProvider } from "../../../components/ui/ConfirmDialog";
import { DocumentsView } from "../../../features/knowledge/steward";
import { api } from "../../../lib/api";

// E7：手工单条新建切片 POST /api/operation-knowledge/chunks。
// 命门红线：后端 create handler 不传 status 默认落 active（default_active_status），
// 且 coerce_integrity_against_d2_gate 只在 integrityStatus=="verified" 时降级、完全不碰
// status。所以裸 POST 会绕过待审池直接落活跃池。E7 表单提交 body 必须写死
// status:"draft" + integrityStatus:"needs_review"——人工新建切片也先进待审池，
// AI/人工都不自动核验。测试断言这两个值。

const realFetch = globalThis.fetch;

// 列表 GET /documents → 返回一行，供可选 documentId 下拉用。
function installListFetch() {
  globalThis.fetch = vi.fn(async (url: unknown) => {
    const u = String(url);
    const body = u.includes("/documents")
      ? {
          items: [
            {
              id: "doc-1",
              title: "运营手册 v3",
              summary: "旧摘要",
              domain: "user_operations",
              sourceType: "imported_markdown",
              sourceName: "手册.md",
              status: "active",
              updatedAt: "2026-06-20T00:00:00Z",
            },
          ],
        }
      : { items: [] };
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

function renderView() {
  return render(
    <ConfirmProvider>
      <DocumentsView />
    </ConfirmProvider>,
  );
}

describe("DocumentsView — E7 手工单条新建切片（强制 draft + needs_review 守红线）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    installListFetch();
  });
  afterEach(() => {
    globalThis.fetch = realFetch;
    vi.restoreAllMocks();
  });

  it("manual new chunk POSTs with status=draft + needs_review", async () => {
    const postSpy = vi.spyOn(api, "post").mockResolvedValue({ id: "ch1" } as never);
    const user = userEvent.setup();
    renderView();

    // 列表加载完成。
    await screen.findByText("运营手册 v3");

    // 展开「手工新建切片」表单。
    const openBtn = await screen.findByRole("button", { name: "手工新建切片" });
    await user.click(openBtn);

    // 填 title + body。
    const titleInput = await screen.findByPlaceholderText("知识标题（必填）");
    await user.type(titleInput, "退款政策说明");
    const bodyInput = await screen.findByPlaceholderText("正文内容");
    await user.type(bodyInput, "下单 7 天内可无理由退款。");

    // 提交。
    const submitBtn = await screen.findByRole("button", { name: "新建切片" });
    await user.click(submitBtn);

    await waitFor(() => {
      expect(postSpy).toHaveBeenCalled();
    });
    expect(postSpy).toHaveBeenCalledWith(
      "/api/operation-knowledge/chunks",
      expect.objectContaining({
        title: "退款政策说明",
        status: "draft",
        integrityStatus: "needs_review",
      }),
    );
  });
});

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ConfirmProvider } from "../../../components/ui/ConfirmDialog";
import { DocumentsView } from "../../../features/knowledge/steward";
import { api } from "../../../lib/api";

// E6：文档元数据编辑 PUT /operation-knowledge/documents/:id。
// 后端是 replace_one 整文档替换（crud.rs update_operation_knowledge_document），
// 任何漏传的字段会被清空。命门：点「编辑」先 GET /documents/:id 取完整文档
// （含 rawContent / contentHash / lineIndex / sectionIndex），表单只暴露少数可改
// 字段，但提交 PUT 时把未编辑字段原样带上，绝不让 replace_one 清空 rawContent。

const realFetch = globalThis.fetch;

const DOC_ID = "doc-1";
const RAW_CONTENT = "# 运营手册\n第一章 正文……长文本原文，绝不能被清空。";
const CONTENT_HASH = "sha256:deadbeef";

// 列表 GET /documents → 返回一行（含 productTags/businessTopics，GET 详情不返回这俩）。
function installListFetch() {
  globalThis.fetch = vi.fn(async (url: unknown) => {
    const u = String(url);
    const body = u.includes("/documents")
      ? {
          items: [
            {
              id: DOC_ID,
              title: "运营手册 v3",
              summary: "旧摘要",
              domain: "user_operations",
              sourceType: "imported_markdown",
              sourceName: "手册.md",
              status: "active",
              catalogSummary: "旧目录摘要",
              updatedAt: "2026-06-20T00:00:00Z",
              routingMap: ["定价", "交付"],
              productTags: ["SaaS", "私有化"],
              businessTopics: ["售前"],
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

// GET 详情返回的完整文档（含 rawContent/contentHash/lineIndex/sectionIndex，
// 但不含 productTags/businessTopics）。
const FULL_DETAIL = {
  id: DOC_ID,
  workspaceId: "default",
  accountId: null,
  domain: "user_operations",
  sourceType: "imported_markdown",
  sourceName: "手册.md",
  title: "运营手册 v3",
  summary: "旧摘要",
  catalogSummary: "旧目录摘要",
  routingMap: ["定价", "交付"],
  riskNotes: ["风险A"],
  rawContent: RAW_CONTENT,
  contentHash: CONTENT_HASH,
  lineIndex: [{ line: 1 }],
  sectionIndex: [{ section: "ch1" }],
  status: "active",
  version: 3,
  updatedAt: "2026-06-20T00:00:00Z",
};

function renderView() {
  return render(
    <ConfirmProvider>
      <DocumentsView />
    </ConfirmProvider>,
  );
}

describe("DocumentsView — E6 文档元数据编辑（整替换回填全字段防清空）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    installListFetch();
  });
  afterEach(() => {
    globalThis.fetch = realFetch;
    vi.restoreAllMocks();
  });

  it("document edit PUTs full body including rawContent", async () => {
    const getSpy = vi.spyOn(api, "get").mockResolvedValue(FULL_DETAIL as never);
    const putSpy = vi.spyOn(api, "put").mockResolvedValue({} as never);
    const user = userEvent.setup();
    renderView();

    // 列表加载完成，文档行出现。
    await screen.findByText("运营手册 v3");

    // 点「编辑」→ 先 GET 详情。
    const editBtn = await screen.findByRole("button", { name: "编辑" });
    await user.click(editBtn);
    await waitFor(() => {
      expect(getSpy).toHaveBeenCalledWith(
        `/api/operation-knowledge/documents/${DOC_ID}`,
      );
    });

    // 表单回填后改标题。
    const titleInput = (await screen.findByDisplayValue("运营手册 v3")) as HTMLInputElement;
    await user.clear(titleInput);
    await user.type(titleInput, "运营手册 v4");

    // 提交。
    const saveBtn = await screen.findByRole("button", { name: "保存修改" });
    await user.click(saveBtn);

    await waitFor(() => {
      expect(putSpy).toHaveBeenCalled();
    });
    const [url, body] = putSpy.mock.calls[0] as [string, Record<string, unknown>];
    expect(url).toBe(`/api/operation-knowledge/documents/${DOC_ID}`);
    // 改后的 title + 未编辑的 rawContent 原值（未被清空）+ contentHash 原样带上。
    expect(body).toEqual(
      expect.objectContaining({
        title: "运营手册 v4",
        rawContent: RAW_CONTENT,
        contentHash: CONTENT_HASH,
      }),
    );
    // lineIndex / sectionIndex 原样回带（替换式 PUT 不丢索引）。
    expect(body.lineIndex).toEqual(FULL_DETAIL.lineIndex);
    expect(body.sectionIndex).toEqual(FULL_DETAIL.sectionIndex);
    // productTags / businessTopics 来自列表项（GET 详情不返回这俩）。
    expect(body.productTags).toEqual(["SaaS", "私有化"]);
    expect(body.businessTopics).toEqual(["售前"]);
  });
});

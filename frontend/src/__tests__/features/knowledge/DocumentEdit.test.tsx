import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ConfirmProvider } from "../../../components/ui/ConfirmDialog";
import { DocumentsView } from "../../../features/knowledge/steward";
import { api } from "../../../lib/api";

// Document metadata edits freeze the detail identity/version and send only
// fields that the operator actually changed.

const realFetch = globalThis.fetch;

const DOC_ID = "doc-1";
const RAW_CONTENT = "# immutable source\nsource body";
const CONTENT_HASH = "sha256:deadbeef";

const LIST_ITEM = {
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
};

// 列表 GET /documents → 返回一行（含 productTags/businessTopics，GET 详情不返回这俩）。
function installListFetch(items = [LIST_ITEM]) {
  globalThis.fetch = vi.fn(async (url: unknown) => {
    const u = String(url);
    const body = u.includes("/documents")
      ? {
          items,
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

describe("DocumentsView — E6 文档元数据编辑（version CAS）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    installListFetch();
  });
  afterEach(() => {
    globalThis.fetch = realFetch;
    vi.restoreAllMocks();
  });

  it("sends only the version and dirty metadata fields", async () => {
    const getSpy = vi.spyOn(api, "get").mockResolvedValue({ item: FULL_DETAIL } as never);
    const patchSpy = vi.spyOn(api, "patch").mockResolvedValue({} as never);
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
      expect(patchSpy).toHaveBeenCalled();
    });
    const [url, body] = patchSpy.mock.calls[0] as [string, Record<string, unknown>];
    expect(url).toBe(`/api/operation-knowledge/documents/${DOC_ID}`);
    expect(body).toEqual({ version: 3, title: "运营手册 v4" });
    expect(body).not.toHaveProperty("rawContent");
    expect(body).not.toHaveProperty("contentHash");
    expect(body).not.toHaveProperty("lineIndex");
    expect(body).not.toHaveProperty("sectionIndex");
  });

  it("rejects a mismatched or versionless detail envelope", async () => {
    vi.spyOn(api, "get").mockResolvedValue({
      item: { ...FULL_DETAIL, id: "other", version: undefined },
    } as never);
    const patchSpy = vi.spyOn(api, "patch").mockResolvedValue({} as never);
    const user = userEvent.setup();
    renderView();

    await screen.findByText("运营手册 v3");
    await user.click(await screen.findByRole("button", { name: "编辑" }));

    await screen.findByText(/文档详情响应与当前文档不匹配/);
    expect(screen.queryByRole("button", { name: "保存修改" })).not.toBeInTheDocument();
    expect(patchSpy).not.toHaveBeenCalled();
  });

  it("closes an unchanged edit without issuing a PATCH", async () => {
    vi.spyOn(api, "get").mockResolvedValue({ item: FULL_DETAIL } as never);
    const patchSpy = vi.spyOn(api, "patch").mockResolvedValue({} as never);
    const user = userEvent.setup();
    renderView();

    await screen.findByText("运营手册 v3");
    await user.click(await screen.findByRole("button", { name: "编辑" }));
    await user.click(await screen.findByRole("button", { name: "保存修改" }));

    await waitFor(() => {
      expect(screen.queryByRole("button", { name: "保存修改" })).not.toBeInTheDocument();
    });
    expect(patchSpy).not.toHaveBeenCalled();
  });

  it("discards a late detail response after another document is selected", async () => {
    const second = { ...LIST_ITEM, id: "doc-2", title: "第二份文档" };
    installListFetch([LIST_ITEM, second]);
    let resolveFirst!: (value: unknown) => void;
    let resolveSecond!: (value: unknown) => void;
    const first = new Promise((resolve) => { resolveFirst = resolve; });
    const latest = new Promise((resolve) => { resolveSecond = resolve; });
    vi.spyOn(api, "get").mockImplementation((url) => (
      String(url).endsWith("doc-1") ? first : latest
    ) as never);
    const user = userEvent.setup();
    renderView();

    await screen.findByText("第二份文档");
    const editButtons = await screen.findAllByRole("button", { name: "编辑" });
    await user.click(editButtons[0]);
    await user.click(editButtons[1]);
    resolveSecond({ item: { ...FULL_DETAIL, id: "doc-2", title: "第二份文档", version: 9 } });
    await screen.findByDisplayValue("第二份文档");
    resolveFirst({ item: FULL_DETAIL });

    await waitFor(() => expect(screen.getByDisplayValue("第二份文档")).toBeInTheDocument());
    expect(screen.queryByDisplayValue("运营手册 v3")).not.toBeInTheDocument();
  });
});

import { render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ConfirmProvider } from "../../../components/ui/ConfirmDialog";
import { ToastProvider } from "../../../components/ui/Toast";
import { FormDialogProvider } from "../../../components/ui/FormDialog";
import { ChunkInspectorPane } from "../../../features/knowledge/shared";

// Task5（F22 接入 + F12 provenance）：ChunkInspectorPane
//  - needs_review chunk → 显「AI 修复建议」入口（挂 ChunkRepairPanel）。
//  - chunk.provenance 非空 → 渲染来源区（source / editedBy）；null 时不崩、不显来源区。

const realFetch = globalThis.fetch;
const CHUNK_ID = "chunk-x";

// 给单条 chunk 装 fetch：/lock 返回 self 占位，/chunks 返回该 chunk。
function installFetch(chunk: Record<string, unknown>) {
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
    const body = { items: [chunk] };
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
          <ChunkInspectorPane chunkId={CHUNK_ID} onClose={() => {}} onClear={() => {}} />
        </FormDialogProvider>
      </ToastProvider>
    </ConfirmProvider>,
  );
}

describe("ChunkInspectorPane — Task5 修复入口 + provenance 来源", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });
  afterEach(() => {
    globalThis.fetch = realFetch;
  });

  it("F22: needs_review chunk 显「AI 修复建议」入口", async () => {
    installFetch({ id: CHUNK_ID, title: "待复核知识", integrityStatus: "needs_review" });
    renderInspector();
    expect(await screen.findByRole("button", { name: "AI 修复建议" })).toBeInTheDocument();
  });

  it("F22: verified chunk 不显修复入口", async () => {
    installFetch({ id: CHUNK_ID, title: "已核验知识", integrityStatus: "verified" });
    renderInspector();
    // 等 chunk 标题渲染出来再断言入口缺席，避免误判 still-loading。
    await screen.findByText("已核验知识");
    expect(screen.queryByRole("button", { name: "AI 修复建议" })).toBeNull();
  });

  it("F12: chunk.provenance 非空时渲染来源区（source / editedBy）", async () => {
    installFetch({
      id: CHUNK_ID,
      title: "有来源知识",
      integrityStatus: "needs_review",
      provenance: { source: "ai_repair", editedBy: "admin1", editedAt: "2026-06-27T00:00:00Z" },
    });
    renderInspector();
    await screen.findByText("有来源知识");
    expect(screen.getByText("来源")).toBeInTheDocument();
    expect(screen.getByText("ai_repair")).toBeInTheDocument();
    expect(screen.getByText("编辑者：admin1")).toBeInTheDocument();
  });

  it("F12: provenance 为 null 时不崩、不显来源区", async () => {
    installFetch({ id: CHUNK_ID, title: "无来源知识", integrityStatus: "verified", provenance: null });
    renderInspector();
    await screen.findByText("无来源知识");
    expect(screen.queryByText("来源")).toBeNull();
  });
});

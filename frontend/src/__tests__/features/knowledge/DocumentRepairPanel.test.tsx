import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { DocumentRepairPanel } from "../../../features/knowledge/DocumentRepairPanel";

function mockChunks(items: unknown[]) {
  globalThis.fetch = vi.fn(async (url: unknown) => {
    const u = String(url);
    const body = u.includes("/documents/") && u.includes("/chunks") ? { items } : {};
    return { ok: true, status: 200, async json() { return body; }, async text() { return JSON.stringify(body); } } as unknown as Response;
  }) as typeof fetch;
}

describe("DocumentRepairPanel 文档级批量修复", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("只渲染 needs_review 的 chunk（过滤 verified/draft）", async () => {
    mockChunks([
      { id: "c1", documentId: "d1", title: "待修切片A", integrityStatus: "needs_review" },
      { id: "c2", documentId: "d1", title: "已核验切片B", integrityStatus: "verified" },
      { id: "c3", documentId: "d1", title: "待修切片C", integrityStatus: "needs_review" },
    ]);
    render(<DocumentRepairPanel documentId="d1" documentTitle="文档一" />);
    await waitFor(() => expect(screen.getByText("待修切片A")).toBeInTheDocument());
    expect(screen.getByText("待修切片C")).toBeInTheDocument();
    expect(screen.queryByText("已核验切片B")).not.toBeInTheDocument();
  });

  it("无 needs_review chunk 时显示空态，不崩", async () => {
    mockChunks([{ id: "c2", documentId: "d1", title: "B", integrityStatus: "verified" }]);
    render(<DocumentRepairPanel documentId="d1" />);
    await waitFor(() => expect(screen.getByText(/无待修切片/)).toBeInTheDocument());
  });

  it("加载失败显示错误态，不静默空", async () => {
    globalThis.fetch = vi.fn(async () => ({
      ok: false, status: 500, async json() { return { error: "boom" }; }, async text() { return "boom"; },
    } as unknown as Response)) as typeof fetch;
    render(<DocumentRepairPanel documentId="d1" />);
    await waitFor(() => expect(screen.getByText(/加载失败/)).toBeInTheDocument());
  });
});

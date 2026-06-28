import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { DocumentRepairPanel } from "../../../features/knowledge/DocumentRepairPanel";

// mock 复用的单 chunk 修复面板：渲染一个能手动触发 onApplied 的按钮，
// 用于验证 DocumentRepairPanel 的 onApplied→onRepaired 透传（不拉真实修复流程）。
vi.mock("../../../features/knowledge/ChunkRepairPanel", () => ({
  ChunkRepairPanel: ({ chunkId, onApplied }: { chunkId: string; onApplied: () => void }) => (
    <button type="button" data-testid={`apply-${chunkId}`} onClick={onApplied}>
      模拟落库
    </button>
  ),
}));

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

  it("单 chunk 落库后触发 onRepaired 回调（供父列表刷新进度）", async () => {
    mockChunks([{ id: "c1", documentId: "d1", title: "待修切片A", integrityStatus: "needs_review" }]);
    const onRepaired = vi.fn();
    render(<DocumentRepairPanel documentId="d1" onRepaired={onRepaired} />);
    await waitFor(() => expect(screen.getByText("待修切片A")).toBeInTheDocument());
    // 展开该 chunk → 渲染 mock 的 ChunkRepairPanel → 模拟落库触发 onApplied
    fireEvent.click(screen.getByText("待修切片A"));
    fireEvent.click(await screen.findByTestId("apply-c1"));
    expect(onRepaired).toHaveBeenCalledTimes(1);
  });
});

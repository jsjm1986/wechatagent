import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ChunkRepairPanel } from "../../../features/knowledge/ChunkRepairPanel";

const PROPOSAL = {
  chunkId: "c1", sessionId: "s1", turn: 1,
  interpretation: { domain: "B2B SaaS", audience: "采购决策人" },
  patch: { summary: "AI改的summary", sourceQuote: "AI脑补quote" },
  missingFields: [{ field: "sourceQuote", reason: "原文无可核验出处" }],
  followupQuestions: [],
  stillMissing: [],
  confidenceHint: 65,
};
const okResp = (body: unknown) => ({ ok: true, status: 200, json: async () => body }) as Response;

let fetchSpy: ReturnType<typeof vi.fn>;
beforeEach(() => { fetchSpy = vi.fn(); vi.stubGlobal("fetch", fetchSpy); });
afterEach(() => { vi.unstubAllGlobals(); });

describe("ChunkRepairPanel", () => {
  it("点AI修复建议→propose→展示patch逐字段+复选框+confidenceHint", async () => {
    fetchSpy.mockResolvedValueOnce(okResp(PROPOSAL));
    render(<ChunkRepairPanel chunkId="c1" originalChunk={{ summary: "原" }} onApplied={vi.fn()} />);
    fireEvent.click(screen.getByText(/AI 修复建议/));
    await waitFor(() => screen.getByText(/AI改的summary/));
    // patch 两字段各一复选框
    expect(screen.getByText("summary")).toBeInTheDocument();
    expect(screen.getByText("sourceQuote")).toBeInTheDocument();
    expect(screen.getAllByRole("checkbox").length).toBe(2);
    // confidenceHint 展示
    expect(screen.getByText(/65/)).toBeInTheDocument();
    // propose 端点对
    expect(fetchSpy.mock.calls[0][0]).toContain("/api/operation-knowledge/chunks/c1/repair");
  });

  it("propose失败（含BudgetExceeded走非2xx）显错误横幅不崩", async () => {
    fetchSpy.mockResolvedValueOnce({ ok: false, status: 429, json: async () => ({}) } as Response);
    render(<ChunkRepairPanel chunkId="c1" originalChunk={{}} onApplied={vi.fn()} />);
    fireEvent.click(screen.getByText(/AI 修复建议/));
    await waitFor(() => screen.getByText(/修复.*失败/));
  });
});

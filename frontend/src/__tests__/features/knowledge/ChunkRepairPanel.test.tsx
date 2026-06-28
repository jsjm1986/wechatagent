import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ChunkRepairPanel } from "../../../features/knowledge/ChunkRepairPanel";
import { applyAiRepairPatch } from "../../../lib/applyAiRepairPatch";
vi.mock("../../../lib/applyAiRepairPatch", () => ({ applyAiRepairPatch: vi.fn() }));

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

  it("有followupQuestions→填答→answer→刷新patch", async () => {
    const withFollowup = { ...PROPOSAL, followupQuestions: [{ id: "q1", field: "sourceQuote", question: "原文哪段支持？" }] };
    const answered = { ...PROPOSAL, turn: 2, patch: { summary: "改进后summary", sourceQuote: "运营补充的quote" }, followupQuestions: [] };
    fetchSpy.mockResolvedValueOnce(okResp(withFollowup)).mockResolvedValueOnce(okResp(answered));
    render(<ChunkRepairPanel chunkId="c1" originalChunk={{}} onApplied={vi.fn()} />);
    fireEvent.click(screen.getByText(/AI 修复建议/));
    await waitFor(() => screen.getByText(/原文哪段支持/));
    // 填答
    fireEvent.change(screen.getByPlaceholderText(/回答/), { target: { value: "第三段" } });
    fireEvent.click(screen.getByText(/提交回答/));
    // answer 端点对 + 刷新 patch
    await waitFor(() => screen.getByText(/运营补充的quote/));
    const [answerUrl, answerInit] = fetchSpy.mock.calls[1];
    expect(answerUrl).toContain("/chunks/c1/repair/answer");
    const body = JSON.parse(answerInit.body);
    expect(body.sessionId).toBe("s1");
    expect(body.turn).toBe(1);
    expect(body.answers[0]).toMatchObject({ id: "q1", field: "sourceQuote", text: "第三段" });
    expect(body.previousPatch).toBeDefined();
  });

  it("落库：勾选字段调applyAiRepairPatch→done+onApplied", async () => {
    fetchSpy.mockResolvedValueOnce(okResp(PROPOSAL));
    (applyAiRepairPatch as ReturnType<typeof vi.fn>).mockResolvedValue({ ok: true });
    const onApplied = vi.fn();
    render(<ChunkRepairPanel chunkId="c1" originalChunk={{ summary: "原" }} onApplied={onApplied} />);
    fireEvent.click(screen.getByText(/AI 修复建议/));
    await waitFor(() => screen.getByText(/AI改的summary/));
    fireEvent.click(screen.getByText(/落库/));
    await waitFor(() => screen.getByText(/已落库/));
    expect(applyAiRepairPatch).toHaveBeenCalledWith(expect.objectContaining({
      chunkId: "c1", sessionId: "s1", turn: 1, confidenceHint: 65,
      acceptedFieldNames: expect.arrayContaining(["summary", "sourceQuote"]),
    }));
    expect(onApplied).toHaveBeenCalled();
  });

  it("落库失败显错误不调onApplied", async () => {
    fetchSpy.mockResolvedValueOnce(okResp(PROPOSAL));
    (applyAiRepairPatch as ReturnType<typeof vi.fn>).mockResolvedValue({ ok: false, reason: "apply_failed" });
    const onApplied = vi.fn();
    render(<ChunkRepairPanel chunkId="c1" originalChunk={{}} onApplied={onApplied} />);
    fireEvent.click(screen.getByText(/AI 修复建议/));
    await waitFor(() => screen.getByText(/AI改的summary/));
    fireEvent.click(screen.getByText(/落库/));
    await waitFor(() => screen.getByText(/落库失败|失败/));
    expect(onApplied).not.toHaveBeenCalled();
  });
});

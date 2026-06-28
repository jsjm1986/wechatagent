import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import CampaignBoard from "../../../features/campaign/CampaignBoard";
import { useCampaignStore } from "../../../stores/campaignStore";
import type { CampaignReport } from "../../../stores/campaignStore";

vi.mock("../../../stores/campaignStore");

function makeReport(n: number): CampaignReport {
  return {
    campaignId: "c1", title: "T", status: "completed",
    summary: { targetCount: n, sent: n, pending: 0, skipped: 0, unknown: 0, blocked: {}, canceled: {}, escalated: {} },
    items: Array.from({ length: n }, (_, i) => ({ contactWxid: `wx_${i}`, name: `n${i}`, status: "sent" })),
  };
}

function mockStore(report: CampaignReport, page: number, setPage: () => void) {
  (useCampaignStore as any).mockImplementation((sel?: any) => {
    const state = { selectedCampaignId: "c1", report, loadReport: vi.fn(), loading: false, lastAttemptedId: "c1", page, setPage };
    return sel ? sel(state) : state;
  });
}

describe("看板翻页", () => {
  beforeEach(() => vi.clearAllMocks());

  it("items > 50 只渲一页 50 行 + 翻页器", () => {
    mockStore(makeReport(120), 0, vi.fn());
    render(<CampaignBoard />);
    expect(screen.getAllByTestId("detail-row")).toHaveLength(50);
    expect(screen.getByText("1 / 3")).toBeInTheDocument();
  });

  it("点下一页调 setPage(1)", () => {
    const setPage = vi.fn();
    mockStore(makeReport(120), 0, setPage);
    render(<CampaignBoard />);
    fireEvent.click(screen.getByText("下一页"));
    expect(setPage).toHaveBeenCalledWith(1);
  });

  it("items <= 50 不渲翻页器", () => {
    mockStore(makeReport(10), 0, vi.fn());
    render(<CampaignBoard />);
    expect(screen.queryByText(/\/ \d/)).toBeNull();
  });
});

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import CampaignList from "../../../features/campaign/CampaignList";
import { useCampaignStore } from "../../../stores/campaignStore";

const openReport = vi.fn();
const setView = vi.fn();
const loadCampaigns = vi.fn();

function mockStore(over: Record<string, unknown>) {
  (useCampaignStore as any).mockImplementation((sel: any) =>
    sel({ campaigns: [], listLoaded: true, loadCampaigns, setView, openReport, ...over }),
  );
}
vi.mock("../../../stores/campaignStore");

beforeEach(() => vi.clearAllMocks());

describe("CampaignList 列表", () => {
  it("空 campaigns 渲空态", () => {
    mockStore({ campaigns: [] });
    render(<CampaignList />);
    expect(screen.getByText("还没有活动")).toBeInTheDocument();
  });

  it("有 campaigns 渲行数 = 长度 + 列头含「已扇出」文案", () => {
    mockStore({ campaigns: [
      { campaignId: "c1", title: "活动一", status: "completed", targetCount: 100, dispatchedCount: 90, createdBy: "admin", createdAt: "2026-06-28T10:00:00Z" },
      { campaignId: "c2", title: "活动二", status: "draft", dispatchedCount: 0, createdBy: "admin", createdAt: "2026-06-28T11:00:00Z" },
    ] });
    render(<CampaignList />);
    expect(screen.getAllByTestId("campaign-row")).toHaveLength(2);
    expect(screen.getByText("已扇出")).toBeInTheDocument();   // A: 文案区分
    expect(screen.getByText("活动一")).toBeInTheDocument();
    // draft 无 targetCount → 渲 —
    expect(screen.getByText("—")).toBeInTheDocument();
  });

  it("点行触发 openReport(切看板)", () => {
    mockStore({ campaigns: [{ campaignId: "c1", title: "活动一", status: "completed", targetCount: 100, dispatchedCount: 90, createdBy: "a", createdAt: "2026-06-28T10:00:00Z" }] });
    render(<CampaignList />);
    fireEvent.click(screen.getAllByTestId("campaign-row")[0]);
    expect(openReport).toHaveBeenCalledWith("c1");
  });

  it("点新建活动切 create 视图", () => {
    mockStore({ campaigns: [] });
    render(<CampaignList />);
    fireEvent.click(screen.getByText("新建活动"));
    expect(setView).toHaveBeenCalledWith("create");
  });

  it("未加载时触发 loadCampaigns（且失败后不循环：listLoaded=true 即不再调）", () => {
    mockStore({ campaigns: [], listLoaded: false });
    render(<CampaignList />);
    expect(loadCampaigns).toHaveBeenCalledTimes(1);
  });
});

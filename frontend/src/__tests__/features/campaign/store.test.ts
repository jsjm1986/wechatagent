import { describe, it, expect, vi, beforeEach } from "vitest";
import { useCampaignStore } from "../../../stores/campaignStore";
import type { CampaignReport } from "../../../stores/campaignStore";
import { api } from "../../../lib/api";

vi.mock("../../../lib/api", () => ({
  api: { get: vi.fn() },
}));
vi.mock("../../../stores/navigationStore", () => ({
  useNavigationStore: { getState: () => ({ setChannel: vi.fn() }) },
}));
vi.mock("../../../stores/uiStore", () => ({
  useUiStore: { getState: () => ({ setError: vi.fn() }) },
}));

const sample: CampaignReport = {
  campaignId: "c1",
  title: "双11老客续费7折",
  status: "completed",
  summary: {
    targetCount: 3, sent: 1, pending: 1, skipped: 0, unknown: 0,
    blocked: { daily_limit: 1 }, canceled: {}, escalated: {},
  },
  items: [
    { contactWxid: "a", name: "张三", status: "sent" },
    { contactWxid: "b", name: "李四", status: "pending" },
    { contactWxid: "c", name: "王五", status: "blocked", reason: "daily_limit" },
  ],
};

describe("campaignStore", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useCampaignStore.setState({ selectedCampaignId: null, report: null, loading: false });
  });

  it("loadReport 成功写入 report 并清 loading", async () => {
    (api.get as any).mockResolvedValue(sample);
    await useCampaignStore.getState().loadReport("c1");
    const s = useCampaignStore.getState();
    expect(s.report).toEqual(sample);
    expect(s.loading).toBe(false);
    expect(api.get).toHaveBeenCalledWith("/api/campaigns/c1/sends");
  });

  it("loadReport 失败时不抛、loading 归位、report 保持 null", async () => {
    (api.get as any).mockRejectedValue(new Error("boom"));
    await useCampaignStore.getState().loadReport("c1");
    const s = useCampaignStore.getState();
    expect(s.report).toBeNull();
    expect(s.loading).toBe(false);
  });

  it("openReport 设置 selectedCampaignId 并触发加载", async () => {
    (api.get as any).mockResolvedValue(sample);
    useCampaignStore.getState().openReport("c1");
    expect(useCampaignStore.getState().selectedCampaignId).toBe("c1");
    // openReport 内部 void loadReport——等微任务跑完
    await Promise.resolve();
    await Promise.resolve();
    expect(api.get).toHaveBeenCalledWith("/api/campaigns/c1/sends");
  });

  it("clear 重置全部", () => {
    useCampaignStore.setState({ selectedCampaignId: "x", report: sample, loading: true });
    useCampaignStore.getState().clear();
    const s = useCampaignStore.getState();
    expect(s.selectedCampaignId).toBeNull();
    expect(s.report).toBeNull();
    expect(s.loading).toBe(false);
  });
});

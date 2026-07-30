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
    useCampaignStore.setState({ selectedCampaignId: null, report: null, loading: false, lastAttemptedId: null });
  });

  it("loadReport 成功写入 report 并清 loading", async () => {
    useCampaignStore.setState({ selectedCampaignId: "c1" });
    (api.get as any).mockResolvedValue(sample);
    await useCampaignStore.getState().loadReport("c1");
    const s = useCampaignStore.getState();
    expect(s.report).toEqual(sample);
    expect(s.loading).toBe(false);
    expect(api.get).toHaveBeenCalledWith("/api/campaigns/c1/sends");
  });

  it("loadReport 失败时不抛、loading 归位、report 保持 null", async () => {
    useCampaignStore.setState({ selectedCampaignId: "c1" });
    (api.get as any).mockRejectedValue(new Error("boom"));
    await useCampaignStore.getState().loadReport("c1");
    const s = useCampaignStore.getState();
    expect(s.report).toBeNull();
    expect(s.loading).toBe(false);
  });

  it("loadReport 失败也会记录 lastAttemptedId（守卫用）", async () => {
    useCampaignStore.setState({ selectedCampaignId: "c1" });
    (api.get as any).mockRejectedValue(new Error("boom"));
    await useCampaignStore.getState().loadReport("c1");
    expect(useCampaignStore.getState().lastAttemptedId).toBe("c1");
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

  it("A 慢 B 快：A 迟到响应不能覆盖 B 报告或 loading", async () => {
    let resolveA!: (value: CampaignReport) => void;
    let resolveB!: (value: CampaignReport) => void;
    const pendingA = new Promise<CampaignReport>((resolve) => { resolveA = resolve; });
    const pendingB = new Promise<CampaignReport>((resolve) => { resolveB = resolve; });
    (api.get as any)
      .mockReturnValueOnce(pendingA)
      .mockReturnValueOnce(pendingB);
    const reportB: CampaignReport = { ...sample, campaignId: "c2", title: "B" };

    useCampaignStore.getState().openReport("c1");
    useCampaignStore.getState().openReport("c2");
    resolveB(reportB);
    await pendingB;
    await Promise.resolve();
    expect(useCampaignStore.getState().report).toEqual(reportB);
    expect(useCampaignStore.getState().loading).toBe(false);

    resolveA(sample);
    await pendingA;
    await Promise.resolve();
    expect(useCampaignStore.getState().selectedCampaignId).toBe("c2");
    expect(useCampaignStore.getState().report).toEqual(reportB);
    expect(useCampaignStore.getState().loading).toBe(false);
  });

  it("响应 campaignId 与请求不一致时拒绝提交", async () => {
    useCampaignStore.setState({ selectedCampaignId: "c1" });
    (api.get as any).mockResolvedValue({ ...sample, campaignId: "wrong" });
    await useCampaignStore.getState().loadReport("c1");
    expect(useCampaignStore.getState().report).toBeNull();
    expect(useCampaignStore.getState().loading).toBe(false);
  });
});

describe("campaignStore 列表/视图扩展", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useCampaignStore.setState({
      selectedCampaignId: null, report: null, loading: false, lastAttemptedId: null,
      view: "list", campaigns: [], listLoading: false, listLoaded: false, page: 0,
    });
  });

  it("setView 切换视图", () => {
    useCampaignStore.getState().setView("create");
    expect(useCampaignStore.getState().view).toBe("create");
  });

  it("loadCampaigns 成功写入 campaigns + listLoaded", async () => {
    const items = [{ campaignId: "c1", title: "T", status: "completed", dispatchedCount: 5, createdBy: "a" }];
    (api.get as any).mockResolvedValue({ items });
    await useCampaignStore.getState().loadCampaigns();
    const s = useCampaignStore.getState();
    expect(s.campaigns).toEqual(items);
    expect(s.listLoaded).toBe(true);
    expect(s.listLoading).toBe(false);
    expect(api.get).toHaveBeenCalledWith("/api/campaigns");
  });

  it("loadCampaigns 失败也置 listLoaded=true(防重试循环) + campaigns 保持空", async () => {
    (api.get as any).mockRejectedValue(new Error("boom"));
    await useCampaignStore.getState().loadCampaigns();
    const s = useCampaignStore.getState();
    expect(s.listLoaded).toBe(true);
    expect(s.listLoading).toBe(false);
    expect(s.campaigns).toEqual([]);
  });

  it("openReport 多设 view=board + page=0", () => {
    (api.get as any).mockResolvedValue({ campaignId: "c1", title: "", status: "", summary: {}, items: [] });
    useCampaignStore.setState({ page: 7 });
    useCampaignStore.getState().openReport("c1");
    const s = useCampaignStore.getState();
    expect(s.view).toBe("board");
    expect(s.page).toBe(0);
    expect(s.selectedCampaignId).toBe("c1");
  });

  it("setPage 改翻页", () => {
    useCampaignStore.getState().setPage(3);
    expect(useCampaignStore.getState().page).toBe(3);
  });

  it("clear 重置新字段", () => {
    useCampaignStore.setState({ view: "board", campaigns: [{ campaignId: "x", title: "", status: "", dispatchedCount: 0, createdBy: "" }], listLoaded: true, page: 5 });
    useCampaignStore.getState().clear();
    const s = useCampaignStore.getState();
    expect(s.view).toBe("list");
    expect(s.campaigns).toEqual([]);
    expect(s.listLoaded).toBe(false);
    expect(s.page).toBe(0);
  });
});

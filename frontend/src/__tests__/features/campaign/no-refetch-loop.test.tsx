import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor } from "@testing-library/react";
import CampaignFeature from "../../../features/campaign";
import { useCampaignStore } from "../../../stores/campaignStore";
import { api } from "../../../lib/api";

vi.mock("../../../lib/api", () => ({ api: { get: vi.fn() } }));
vi.mock("../../../stores/navigationStore", () => ({
  useNavigationStore: { getState: () => ({ setChannel: vi.fn() }) },
}));
vi.mock("../../../stores/uiStore", () => ({
  useUiStore: { getState: () => ({ setError: vi.fn() }) },
}));

describe("看板加载失败不进入重试循环", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useCampaignStore.setState({
      selectedCampaignId: null, report: null, loading: false, lastAttemptedId: null, view: "board",
    });
  });

  it("加载失败后 effect 不再自动重发请求（lastAttemptedId 守卫）", async () => {
    (api.get as any).mockRejectedValue(new Error("boom"));
    // 模拟「直接切到该频道、未经 openReport」：只设 selectedCampaignId（index 路由壳已置 view:"board" 渲看板）
    useCampaignStore.setState({ selectedCampaignId: "c1" });

    render(<CampaignFeature />);

    // 等到第一次请求发生 + 失败 + loading 归位
    await waitFor(() => expect(api.get).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(useCampaignStore.getState().loading).toBe(false));

    // 关键断言：再等若干微/宏任务，请求次数仍为 1，没有循环重发
    await new Promise((r) => setTimeout(r, 50));
    expect(api.get).toHaveBeenCalledTimes(1);
    expect(useCampaignStore.getState().lastAttemptedId).toBe("c1");
    expect(useCampaignStore.getState().report).toBeNull();
  });
});

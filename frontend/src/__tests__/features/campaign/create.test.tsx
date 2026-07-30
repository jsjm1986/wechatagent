import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import CampaignCreate from "../../../features/campaign/CampaignCreate";
import { api } from "../../../lib/api";

vi.mock("../../../lib/api", () => ({ api: { get: vi.fn(), post: vi.fn(), patch: vi.fn() } }));
const setView = vi.fn();
const openReport = vi.fn();
vi.mock("../../../stores/campaignStore", () => ({
  useCampaignStore: (sel: any) => sel({ setView, openReport }),
}));
vi.mock("../../../stores/uiStore", () => ({
  useUiStore: (sel: any) => sel({ setError: vi.fn() }),
}));

beforeEach(() => {
  vi.clearAllMocks();
  (api.get as any).mockResolvedValue({ items: [] });
});

describe("CampaignCreate 建活动表单", () => {
  it("标题/意图空时圈人预览按钮 disabled", () => {
    render(<CampaignCreate />);
    const btn = screen.getByText("圈人预览") as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });

  it("填表点预览 → 调 create + preview，显示命中数", async () => {
    (api.post as any)
      .mockResolvedValueOnce({ id: "c_new", specVersion: 1, specHash: "h1" })
      .mockResolvedValueOnce({ campaignId: "c_new", specVersion: 1, specHash: "h1", targetCount: 42, samples: [{ wxid: "wx1", name: "张三" }] });
    render(<CampaignCreate />);
    fireEvent.change(screen.getByPlaceholderText(/双11/), { target: { value: "活动A" } });
    fireEvent.change(screen.getByPlaceholderText(/活动要点/), { target: { value: "7折" } });
    fireEvent.click(screen.getByText("圈人预览"));
    await waitFor(() => expect(screen.getByText(/命中 42 人/)).toBeInTheDocument());
    expect(api.post).toHaveBeenNthCalledWith(1, "/api/campaigns", expect.objectContaining({ title: "活动A", intentText: "7折" }));
    expect(api.post).toHaveBeenNthCalledWith(2, "/api/campaigns/c_new/preview", {});
  });

  it("改条件再预览 → 先 CAS 保存完整 draft，再按新版本 preview", async () => {
    (api.post as any)
      .mockResolvedValueOnce({ id: "c_new", specVersion: 1, specHash: "h1" })
      .mockResolvedValueOnce({ campaignId: "c_new", specVersion: 1, specHash: "h1", targetCount: 42, samples: [] })
      .mockResolvedValueOnce({ campaignId: "c_new", specVersion: 2, specHash: "h2", targetCount: 8, samples: [] });
    (api.patch as any).mockResolvedValueOnce({ campaignId: "c_new", specVersion: 2, specHash: "h2" });
    render(<CampaignCreate />);
    fireEvent.change(screen.getByPlaceholderText(/双11/), { target: { value: "活动A" } });
    fireEvent.change(screen.getByPlaceholderText(/活动要点/), { target: { value: "7折" } });
    fireEvent.click(screen.getByText("圈人预览"));
    await waitFor(() => expect(screen.getByText(/命中 42 人/)).toBeInTheDocument());
    // 改售后条件（作废 preview），再预览。三个 select（客户阶段/售后/价值分层）默认都显示"不限"，
    // 用唯一的 in_aftercare 选项精确定位售后 select，避免 getByDisplayValue 的歧义。
    const aftercareSelect = screen
      .getAllByRole("combobox")
      .find((el) => el.querySelector('option[value="in_aftercare"]'))!;
    fireEvent.change(aftercareSelect, { target: { value: "in_aftercare" } });
    fireEvent.click(screen.getByText("圈人预览"));
    await waitFor(() => expect(screen.getByText(/命中 8 人/)).toBeInTheDocument());
    expect(api.patch).toHaveBeenCalledWith("/api/campaigns/c_new", expect.objectContaining({
      title: "活动A",
      intentText: "7折",
      segmentFilter: { aftercare: "in_aftercare" },
      expectedSpecVersion: 1,
    }));
    // create 仅 1 次；第二次 preview 的 POST 必须发生在 PATCH 成功之后。
    const createCalls = (api.post as any).mock.calls.filter((c: any[]) => c[0] === "/api/campaigns");
    expect(createCalls).toHaveLength(1);
    expect(api.post).toHaveBeenLastCalledWith("/api/campaigns/c_new/preview", {});
  });

  it("命中 0 人显示提示", async () => {
    (api.post as any)
      .mockResolvedValueOnce({ id: "c0", specVersion: 1, specHash: "h0" })
      .mockResolvedValueOnce({ campaignId: "c0", specVersion: 1, specHash: "h0", targetCount: 0, samples: [] });
    render(<CampaignCreate />);
    fireEvent.change(screen.getByPlaceholderText(/双11/), { target: { value: "A" } });
    fireEvent.change(screen.getByPlaceholderText(/活动要点/), { target: { value: "x" } });
    fireEvent.click(screen.getByText("圈人预览"));
    await waitFor(() => expect(screen.getByText(/命中 0 人，调整条件/)).toBeInTheDocument());
  });

  it("红线：无 dispatch 按钮/控件", async () => {
    (api.post as any)
      .mockResolvedValueOnce({ id: "c1", specVersion: 1, specHash: "h1" })
      .mockResolvedValueOnce({ campaignId: "c1", specVersion: 1, specHash: "h1", targetCount: 5, samples: [] });
    render(<CampaignCreate />);
    fireEvent.change(screen.getByPlaceholderText(/双11/), { target: { value: "A" } });
    fireEvent.change(screen.getByPlaceholderText(/活动要点/), { target: { value: "x" } });
    fireEvent.click(screen.getByText("圈人预览"));
    await waitFor(() => expect(screen.getByText(/命中 5 人/)).toBeInTheDocument());
    // 整个组件不得出现 dispatch/确认推送/推送 触发按钮（只允许"请在 AI 总控对话中 dispatch"提示文字）
    expect(screen.queryByText(/^确认推送$/)).toBeNull();
    expect(screen.queryByText(/^立即推送$/)).toBeNull();
    expect(screen.queryByText(/^dispatch$/i)).toBeNull();
  });
});

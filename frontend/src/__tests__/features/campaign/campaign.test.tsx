import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, within, fireEvent } from "@testing-library/react";
import CampaignFeature from "../../../features/campaign/CampaignBoard";
import { bucketTone, bucketLabel } from "../../../features/campaign/buckets";
import { useCampaignStore } from "../../../stores/campaignStore";
import type { CampaignReport } from "../../../stores/campaignStore";

vi.mock("../../../stores/campaignStore");

const report: CampaignReport = {
  campaignId: "c1",
  title: "双11老客续费7折",
  status: "completed",
  summary: {
    targetCount: 5, sent: 2, pending: 1, skipped: 0, unknown: 0,
    blocked: { daily_limit: 1 }, canceled: {}, escalated: { blocked_unverified_product_claim: 1 },
  },
  items: [
    { contactWxid: "a", name: "张三", status: "sent" },
    { contactWxid: "b", name: "李四", status: "sent" },
    { contactWxid: "c", name: "王五", status: "pending" },
    { contactWxid: "d", name: "赵六", status: "blocked", reason: "daily_limit" },
    { contactWxid: "e", name: "", status: "escalated", reason: "blocked_unverified_product_claim" },
  ],
};

describe("bucketTone / bucketLabel", () => {
  it("7 桶映射到正确 tone", () => {
    expect(bucketTone("sent")).toBe("running");
    expect(bucketTone("pending")).toBe("scheduled");
    expect(bucketTone("blocked")).toBe("blocked");
    expect(bucketTone("escalated")).toBe("held");
    expect(bucketTone("canceled")).toBe("inactive");
    expect(bucketTone("skipped")).toBe("inactive");
    expect(bucketTone("unknown")).toBe("inactive");
    expect(bucketTone("天外飞仙")).toBe("inactive"); // 兜底
  });
  it("7 桶中文标签", () => {
    expect(bucketLabel("sent")).toBe("已送达");
    expect(bucketLabel("escalated")).toBe("已请示");
    expect(bucketLabel("blocked")).toBe("被拦");
    expect(bucketLabel("unknown")).toBe("未知");
    expect(bucketLabel("pending")).toBe("在途");
    expect(bucketLabel("canceled")).toBe("已取消");
    expect(bucketLabel("skipped")).toBe("去重跳过");
  });
});

describe("CampaignFeature", () => {
  beforeEach(() => vi.clearAllMocks());

  it("selectedCampaignId=null 渲 EmptyState", () => {
    (useCampaignStore as any).mockReturnValue({
      selectedCampaignId: null, report: null, loading: false, loadReport: vi.fn(),
    });
    render(<CampaignFeature />);
    expect(screen.getByText("暂无活动结果")).toBeInTheDocument();
  });

  it("有 report 渲汇总数值 + 明细表行数 = items 长度", () => {
    (useCampaignStore as any).mockImplementation((sel?: any) => {
      const state = { selectedCampaignId: "c1", report, loading: false, lastAttemptedId: "c1", loadReport: vi.fn(), page: 0, setPage: vi.fn() };
      return sel ? sel(state) : state;
    });
    render(<CampaignFeature />);
    // 标题
    expect(screen.getByText("双11老客续费7折")).toBeInTheDocument();
    // sent 汇总值 2（用 testid 精确取，避免与表格数字串扰）
    expect(screen.getByTestId("metric-sent")).toHaveTextContent("2");
    expect(screen.getByTestId("metric-pending")).toHaveTextContent("1");
    // escalated reason 二级细分（在 escalated 汇总桶内）
    expect(
      within(screen.getByTestId("metric-escalated")).getByText(/blocked_unverified_product_claim/)
    ).toBeInTheDocument();
    // 明细表行数（tbody tr）= 5
    expect(screen.getAllByTestId("detail-row")).toHaveLength(5);
    // 空 name 行渲 —（最后一行 e 的客户列）
    const rows = screen.getAllByTestId("detail-row");
    expect(rows[4]).toHaveTextContent("—");
  });

  it("空 items 渲明细空态", () => {
    (useCampaignStore as any).mockReturnValue({
      selectedCampaignId: "c1",
      report: { ...report, items: [], summary: { ...report.summary, targetCount: 0, sent: 0, pending: 0, blocked: {}, escalated: {} } },
      loading: false, loadReport: vi.fn(), page: 0, setPage: vi.fn(),
    });
    render(<CampaignFeature />);
    expect(screen.getByText("暂无推送明细")).toBeInTheDocument();
  });

  it("点某桶 chip 后明细表只剩该桶行", () => {
    (useCampaignStore as any).mockImplementation((sel?: any) => {
      const state = { selectedCampaignId: "c1", report, loading: false, lastAttemptedId: "c1", loadReport: vi.fn(), page: 0, setPage: vi.fn() };
      return sel ? sel(state) : state;
    });
    render(<CampaignFeature />);
    // 初始 5 行
    expect(screen.getAllByTestId("detail-row")).toHaveLength(5);
    // 点「被拦」chip（bucketLabel("blocked")）。注意「被拦」在 metric 块和 chip 都出现，
    // 用 getAllByRole("button") 精确取 chip（metric label 是 span 不是 button）。
    const blockedChip = screen.getAllByRole("button").find((b) => b.textContent === "被拦");
    expect(blockedChip).toBeTruthy();
    fireEvent.click(blockedChip!);
    // 只剩 1 行 blocked（赵六）
    const rows = screen.getAllByTestId("detail-row");
    expect(rows).toHaveLength(1);
    expect(rows[0]).toHaveTextContent("赵六");
  });
});

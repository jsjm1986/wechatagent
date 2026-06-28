import { describe, it, expect } from "vitest";
import { campaignStatusTone, campaignStatusLabel } from "../../../features/campaign/CampaignList";

describe("campaignStatusTone / campaignStatusLabel", () => {
  it("6 状态 tone 映射", () => {
    expect(campaignStatusTone("draft")).toBe("inactive");
    expect(campaignStatusTone("previewed")).toBe("scheduled");
    expect(campaignStatusTone("confirmed")).toBe("scheduled");
    expect(campaignStatusTone("dispatching")).toBe("running");
    expect(campaignStatusTone("completed")).toBe("running");
    expect(campaignStatusTone("canceled")).toBe("blocked");
    expect(campaignStatusTone("天外飞仙")).toBe("inactive"); // 兜底
  });
  it("6 状态中文标签 + 未知值返回原值", () => {
    expect(campaignStatusLabel("draft")).toBe("草稿");
    expect(campaignStatusLabel("previewed")).toBe("已预览");
    expect(campaignStatusLabel("confirmed")).toBe("已确认");
    expect(campaignStatusLabel("dispatching")).toBe("推送中");
    expect(campaignStatusLabel("completed")).toBe("已完成");
    expect(campaignStatusLabel("canceled")).toBe("已取消");
    expect(campaignStatusLabel("xyz")).toBe("xyz"); // 诚实兜底
  });
});

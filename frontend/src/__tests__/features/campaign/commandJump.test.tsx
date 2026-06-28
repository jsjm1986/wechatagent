import { describe, it, expect } from "vitest";
import { dispatchCampaignId } from "../../../features/command-center";
import type { CommandToolCall } from "../../../types";

const call = (over: Partial<CommandToolCall>): CommandToolCall => ({
  id: "1", toolName: "wechatagent.dispatch_campaign", status: "succeeded",
  response: { campaignId: "c1" }, ...over,
});

describe("dispatchCampaignId 守卫", () => {
  it("succeeded + campaignId → 返回 id", () => {
    expect(dispatchCampaignId(call({}))).toBe("c1");
  });
  it("executed_unverified + campaignId → 返回 id", () => {
    expect(dispatchCampaignId(call({ status: "executed_unverified" }))).toBe("c1");
  });
  it("非 dispatch_campaign 工具 → null", () => {
    expect(dispatchCampaignId(call({ toolName: "wechatagent.preview_campaign" }))).toBeNull();
  });
  it("dry_run → null（防死链）", () => {
    expect(dispatchCampaignId(call({ status: "dry_run", response: {} }))).toBeNull();
  });
  it("pending_confirmation / 无 response → null", () => {
    expect(dispatchCampaignId(call({ status: "succeeded", response: undefined }))).toBeNull();
  });
  it("campaignId 非字符串 → null", () => {
    expect(dispatchCampaignId(call({ response: { campaignId: 123 } }))).toBeNull();
  });
});

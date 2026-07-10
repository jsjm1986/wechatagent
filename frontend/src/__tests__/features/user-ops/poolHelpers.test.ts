import { describe, it, expect } from "vitest";
import { overdueHours } from "../../../features/user-ops/poolHelpers";

const now = new Date("2026-07-11T12:00:00Z").getTime();

describe("overdueHours", () => {
  it("来了消息之后没回、且超阈值 → 返回小时数", () => {
    const c = { lastInboundAt: "2026-07-10T12:00:00Z", lastOutboundAt: "2026-07-09T00:00:00Z" } as any;
    expect(overdueHours(c, now)).toBe(24);
  });
  it("回过了（outbound 晚于 inbound）→ null", () => {
    const c = { lastInboundAt: "2026-07-10T12:00:00Z", lastOutboundAt: "2026-07-11T00:00:00Z" } as any;
    expect(overdueHours(c, now)).toBeNull();
  });
  it("来了消息但未超阈值 → null", () => {
    const c = { lastInboundAt: "2026-07-11T11:00:00Z", lastOutboundAt: null } as any;
    expect(overdueHours(c, now)).toBeNull(); // 仅 1h < 24h
  });
  it("从没来过消息 → null", () => {
    const c = { lastInboundAt: undefined, lastOutboundAt: undefined } as any;
    expect(overdueHours(c, now)).toBeNull();
  });
});

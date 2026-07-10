import { describe, it, expect } from "vitest";
import { overdueHours, formatRelativeTime } from "../../../features/user-ops/poolHelpers";

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

describe("formatRelativeTime", () => {
  it("30 秒内 → 刚刚", () => {
    expect(formatRelativeTime("2026-07-11T11:59:40Z", now)).toBe("刚刚");
  });
  it("分钟级 → N 分钟前", () => {
    expect(formatRelativeTime("2026-07-11T11:45:00Z", now)).toBe("15 分钟前");
  });
  it("小时级 → N 小时前", () => {
    expect(formatRelativeTime("2026-07-11T09:00:00Z", now)).toBe("3 小时前");
  });
  it("天级 → N 天前", () => {
    expect(formatRelativeTime("2026-07-09T12:00:00Z", now)).toBe("2 天前");
  });
  it("月级 → N 个月前", () => {
    expect(formatRelativeTime("2026-05-01T12:00:00Z", now)).toBe("2 个月前");
  });
  it("缺失/非法 → null", () => {
    expect(formatRelativeTime(null, now)).toBeNull();
    expect(formatRelativeTime("not-a-date", now)).toBeNull();
  });
  it("未来时间（时钟偏移）→ 刚刚", () => {
    expect(formatRelativeTime("2026-07-11T12:05:00Z", now)).toBe("刚刚");
  });
});

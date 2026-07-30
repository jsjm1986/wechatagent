import { describe, it, expect } from "vitest";
import { defaultPolicy, extractPolicy, validatePolicy } from "../../../features/ask-human-config/policyForm";

type PolicyLike = Record<string, unknown>;

describe("defaultPolicy", () => {
  it("空链 + 保守默认开关(safety/product/stuck=true, aiPolicyHold=false), 可选项 undefined", () => {
    const p = defaultPolicy();
    expect(p.deciderChain).toEqual([]);
    expect(p.escalateSafetyGuard).toBe(true);
    expect(p.escalateUnverifiedProduct).toBe(true);
    expect(p.escalateAiPolicyHold).toBe(false);
    expect(p.escalateStuck).toBe(true);
    expect(p.timeoutHours).toBeUndefined();
    expect(p.dedupeWindowHours).toBeUndefined();
    expect(p.dailyPushCap).toBeUndefined();
    expect(p.quietHours).toBeUndefined();
  });
});

describe("extractPolicy", () => {
  it("完整 askHumanPolicy 原样抽出", () => {
    const item = { askHumanPolicy: {
      deciderChain: [{ wxid: "w1", displayName: "老板", accountId: "acc1" }],
      escalateSafetyGuard: false, escalateUnverifiedProduct: true,
      escalateAiPolicyHold: true, escalateStuck: false,
      dedupeWindowHours: 6, dailyPushCap: 3,
      quietHours: { startHour: 22, endHour: 7, tzOffsetHours: 8 }, timeoutHours: 24,
    } };
    const p = extractPolicy(item);
    expect(p.deciderChain).toEqual([{ wxid: "w1", displayName: "老板", accountId: "acc1" }]);
    expect(p.escalateSafetyGuard).toBe(false);
    expect(p.quietHours).toEqual({ startHour: 22, endHour: 7, tzOffsetHours: 8 });
    expect(p.timeoutHours).toBe(24);
  });
  it("askHumanPolicy 缺失/null/非对象 → 回落 defaultPolicy", () => {
    expect(extractPolicy({ askHumanPolicy: null })).toEqual(defaultPolicy());
    expect(extractPolicy({})).toEqual(defaultPolicy());
    expect(extractPolicy(null)).toEqual(defaultPolicy());
    expect(extractPolicy("garbage")).toEqual(defaultPolicy());
  });
  it("部分字段缺 → 缺的补默认, 有的保留", () => {
    const p = extractPolicy({ askHumanPolicy: { deciderChain: [{ wxid: "w1", accountId: "acc1" }] } });
    expect(p.deciderChain).toEqual([{ wxid: "w1", accountId: "acc1" }]);
    expect(p.escalateSafetyGuard).toBe(true);
    expect(p.escalateAiPolicyHold).toBe(false);
    expect(p.timeoutHours).toBeUndefined();
  });
});

describe("validatePolicy", () => {
  const ok: PolicyLike = {
    deciderChain: [{ wxid: "w1", accountId: "acc1" }], escalateSafetyGuard: true, escalateUnverifiedProduct: true,
    escalateAiPolicyHold: false, escalateStuck: true,
  };
  it("合法策略 → 空错误数组", () => {
    expect(validatePolicy(ok as never)).toEqual([]);
  });
  it("空决策人链是显式关闭态", () => {
    expect(validatePolicy({ ...ok, deciderChain: [] } as never)).toEqual([]);
  });
  it("决策人 wxid 空白 → 报错", () => {
    expect(validatePolicy({ ...ok, deciderChain: [{ wxid: "  ", accountId: "acc1" }] } as never).length).toBeGreaterThan(0);
  });
  it("决策人未绑定发送账号 → 报错", () => {
    expect(validatePolicy({ ...ok, deciderChain: [{ wxid: "w1" }] } as never)).toContain("决策人必须绑定发送账号");
  });
  it("quietHours 小时越界(>23) → 报错", () => {
    expect(validatePolicy({ ...ok, quietHours: { startHour: 24, endHour: 7, tzOffsetHours: 8 } } as never).length).toBeGreaterThan(0);
  });
  it("dedupeWindowHours / timeoutHours 负数 → 报错", () => {
    expect(validatePolicy({ ...ok, dedupeWindowHours: -1 } as never).length).toBeGreaterThan(0);
    expect(validatePolicy({ ...ok, timeoutHours: -5 } as never).length).toBeGreaterThan(0);
  });
  it("dailyPushCap < 1 → 报错", () => {
    expect(validatePolicy({ ...ok, dailyPushCap: 0 } as never).length).toBeGreaterThan(0);
  });
});

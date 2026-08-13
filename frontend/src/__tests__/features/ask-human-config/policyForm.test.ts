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

// ── S5-5 预授权底线（standing order）两字段 ──

describe("standing order 字段", () => {
  it("defaultPolicy 不含底线字段（未启用态）", () => {
    const p = defaultPolicy();
    expect(p.standingOrder).toBeUndefined();
    expect(p.standingOrderAfterHours).toBeUndefined();
  });

  it("extractPolicy 抽取合法 standingOrder/standingOrderAfterHours", () => {
    const p = extractPolicy({ askHumanPolicy: {
      deciderChain: [], escalateSafetyGuard: true, escalateUnverifiedProduct: true,
      escalateAiPolicyHold: false, escalateStuck: true,
      standingOrder: "最多 95 折，赠品可送", standingOrderAfterHours: 12,
    } });
    expect(p.standingOrder).toBe("最多 95 折，赠品可送");
    expect(p.standingOrderAfterHours).toBe(12);
  });

  it("extractPolicy 对缺失/非法类型回落 undefined", () => {
    const missing = extractPolicy({ askHumanPolicy: {
      deciderChain: [], escalateSafetyGuard: true, escalateUnverifiedProduct: true,
      escalateAiPolicyHold: false, escalateStuck: true,
    } });
    expect(missing.standingOrder).toBeUndefined();
    expect(missing.standingOrderAfterHours).toBeUndefined();
    const garbage = extractPolicy({ askHumanPolicy: {
      deciderChain: [], escalateSafetyGuard: true, escalateUnverifiedProduct: true,
      escalateAiPolicyHold: false, escalateStuck: true,
      standingOrder: 42, standingOrderAfterHours: "twelve",
    } });
    expect(garbage.standingOrder).toBeUndefined();
    expect(garbage.standingOrderAfterHours).toBeUndefined();
  });

  const okBase: PolicyLike = {
    deciderChain: [], escalateSafetyGuard: true, escalateUnverifiedProduct: true,
    escalateAiPolicyHold: false, escalateStuck: true,
  };

  it("validatePolicy：成对配置且合法 → 通过；两者全缺省 → 通过", () => {
    expect(validatePolicy({ ...okBase, standingOrder: "底线口径", standingOrderAfterHours: 12 } as never)).toEqual([]);
    expect(validatePolicy(okBase as never)).toEqual([]);
  });

  it("validatePolicy：只配一半 → 报错（防配了永不生效）", () => {
    expect(validatePolicy({ ...okBase, standingOrder: "底线口径" } as never).length).toBeGreaterThan(0);
    expect(validatePolicy({ ...okBase, standingOrderAfterHours: 12 } as never).length).toBeGreaterThan(0);
  });

  it("validatePolicy：空白口径 / 超长口径 → 报错", () => {
    expect(validatePolicy({ ...okBase, standingOrder: "   ", standingOrderAfterHours: 12 } as never).length).toBeGreaterThan(0);
    expect(validatePolicy({ ...okBase, standingOrder: "字".repeat(2001), standingOrderAfterHours: 12 } as never).length).toBeGreaterThan(0);
  });

  it("validatePolicy：时限 ≤0 或 >8760 → 报错", () => {
    expect(validatePolicy({ ...okBase, standingOrder: "底线口径", standingOrderAfterHours: 0 } as never).length).toBeGreaterThan(0);
    expect(validatePolicy({ ...okBase, standingOrder: "底线口径", standingOrderAfterHours: -1 } as never).length).toBeGreaterThan(0);
    expect(validatePolicy({ ...okBase, standingOrder: "底线口径", standingOrderAfterHours: 8761 } as never).length).toBeGreaterThan(0);
  });
});

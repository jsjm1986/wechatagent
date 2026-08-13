import type { AskHumanPolicy as AskHumanPolicyBase, DeciderRef, AskHumanQuietHours } from "../../types";

// S5-5 预授权底线两字段（standingOrder / standingOrderAfterHours，对齐后端
// AskHumanPolicy camelCase serde）。配置页本地扩展全局类型，两字段成对配置。
export type AskHumanPolicy = AskHumanPolicyBase & {
  standingOrder?: string;
  standingOrderAfterHours?: number;
};

// 空链 + 保守默认开关（与后端 ResolvedAskHumanPolicy 非-all 模式回落一致）。
export function defaultPolicy(): AskHumanPolicy {
  return {
    deciderChain: [],
    escalateSafetyGuard: true,
    escalateUnverifiedProduct: true,
    escalateAiPolicyHold: false,
    escalateStuck: true,
  };
}

function asBool(v: unknown, fallback: boolean): boolean {
  return typeof v === "boolean" ? v : fallback;
}
function asNumOrUndef(v: unknown): number | undefined {
  return typeof v === "number" && Number.isFinite(v) ? v : undefined;
}

// 从 GET /operation-domains/:domain 的 item.askHumanPolicy 抽策略；缺/非对象 → defaultPolicy()。
// 逐字段存在性回落，保证返回结构完整可编辑。
export function extractPolicy(domainItem: unknown): AskHumanPolicy {
  const raw =
    domainItem && typeof domainItem === "object"
      ? (domainItem as Record<string, unknown>).askHumanPolicy
      : null;
  if (!raw || typeof raw !== "object") return defaultPolicy();
  const p = raw as Record<string, unknown>;
  const d = defaultPolicy();
  const chain: DeciderRef[] = Array.isArray(p.deciderChain)
    ? (p.deciderChain as unknown[]).flatMap((it) => {
        if (!it || typeof it !== "object") return [];
        const wxid = (it as Record<string, unknown>).wxid;
        if (typeof wxid !== "string") return [];
        const dn = (it as Record<string, unknown>).displayName;
        const accountId = (it as Record<string, unknown>).accountId;
        return [{
          wxid,
          ...(typeof dn === "string" ? { displayName: dn } : {}),
          ...(typeof accountId === "string" ? { accountId } : {}),
        }];
      })
    : [];
  let quietHours: AskHumanQuietHours | undefined;
  const qhRaw = p.quietHours;
  if (qhRaw && typeof qhRaw === "object") {
    const q = qhRaw as Record<string, unknown>;
    if (typeof q.startHour === "number" && typeof q.endHour === "number" && typeof q.tzOffsetHours === "number") {
      quietHours = { startHour: q.startHour, endHour: q.endHour, tzOffsetHours: q.tzOffsetHours };
    }
  }
  const dedupe = asNumOrUndef(p.dedupeWindowHours);
  const cap = asNumOrUndef(p.dailyPushCap);
  const timeout = asNumOrUndef(p.timeoutHours);
  const standingOrder = typeof p.standingOrder === "string" ? p.standingOrder : undefined;
  const standingOrderAfterHours = asNumOrUndef(p.standingOrderAfterHours);
  return {
    deciderChain: chain,
    escalateSafetyGuard: asBool(p.escalateSafetyGuard, d.escalateSafetyGuard),
    escalateUnverifiedProduct: asBool(p.escalateUnverifiedProduct, d.escalateUnverifiedProduct),
    escalateAiPolicyHold: asBool(p.escalateAiPolicyHold, d.escalateAiPolicyHold),
    escalateStuck: asBool(p.escalateStuck, d.escalateStuck),
    ...(dedupe !== undefined ? { dedupeWindowHours: dedupe } : {}),
    ...(cap !== undefined ? { dailyPushCap: cap } : {}),
    ...(quietHours ? { quietHours } : {}),
    ...(timeout !== undefined ? { timeoutHours: timeout } : {}),
    ...(standingOrder !== undefined ? { standingOrder } : {}),
    ...(standingOrderAfterHours !== undefined ? { standingOrderAfterHours } : {}),
  };
}

// 校验草稿；返回错误消息数组（空 = 通过）。前端体验校验，后端是权威。
export function validatePolicy(p: AskHumanPolicy): string[] {
  const errs: string[] = [];
  // 空链是后端定义的显式关闭态，不是校验错误。
  for (const d of p.deciderChain ?? []) {
    if (!d.wxid || d.wxid.trim().length === 0) {
      errs.push("决策人 wxid 不能为空");
      break;
    }
    if (!d.accountId || d.accountId.trim().length === 0) {
      errs.push("决策人必须绑定发送账号");
      break;
    }
  }
  if (p.quietHours) {
    const { startHour, endHour } = p.quietHours;
    if (startHour < 0 || startHour > 23 || endHour < 0 || endHour > 23) {
      errs.push("静默时段小时须 0-23");
    }
  }
  if (p.dedupeWindowHours !== undefined && p.dedupeWindowHours < 0) errs.push("去重窗口不能为负");
  if (p.timeoutHours !== undefined && p.timeoutHours < 0) errs.push("超时小时不能为负");
  if (p.dailyPushCap !== undefined && p.dailyPushCap < 1) errs.push("每日上限至少为 1");
  // S5-5 预授权底线：两字段成对；口径非空白且 ≤2000 字符；时限 >0 且 ≤8760（与后端权威校验一致）。
  const hasOrder = p.standingOrder !== undefined;
  const hasHours = p.standingOrderAfterHours !== undefined;
  if (hasOrder !== hasHours) {
    errs.push("预授权底线口径与生效时限必须成对配置（或都留空）");
  } else if (hasOrder && hasHours) {
    if ((p.standingOrder ?? "").trim().length === 0) errs.push("预授权底线口径不能为空白");
    else if ([...(p.standingOrder ?? "")].length > 2000) errs.push("预授权底线口径最长 2000 字符");
    const h = p.standingOrderAfterHours ?? 0;
    if (!(h > 0 && h <= 8760)) errs.push("预授权底线生效时限须大于 0 且不超过 8760 小时");
  }
  return errs;
}

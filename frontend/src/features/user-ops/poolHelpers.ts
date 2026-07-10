import type { Contact } from "../../types";

const OVERDUE_THRESHOLD_HOURS = 24;

/**
 * 超时未跟进派生（纯函数，不需已读系统）：客户来了消息（lastInboundAt）之后
 * 没有出站回复（lastOutboundAt 早于 lastInboundAt 或缺失）、且距今超过阈值 →
 * 返回已过小时数（向下取整）；否则 null。
 */
export function overdueHours(contact: Contact, nowMs: number): number | null {
  if (!contact.lastInboundAt) return null;
  const inbound = new Date(contact.lastInboundAt).getTime();
  if (Number.isNaN(inbound)) return null;
  const outbound = contact.lastOutboundAt ? new Date(contact.lastOutboundAt).getTime() : 0;
  // 已回复（出站不早于入站）→ 不算超时。
  if (outbound >= inbound) return null;
  const hours = Math.floor((nowMs - inbound) / 3_600_000);
  return hours >= OVERDUE_THRESHOLD_HOURS ? hours : null;
}

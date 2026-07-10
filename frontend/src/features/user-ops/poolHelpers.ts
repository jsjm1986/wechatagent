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

/**
 * 相对时间格式化（纯函数）：把 ISO 时间戳转成「刚刚 / N 分钟前 / N 小时前 / N 天前 / N 个月前」。
 * 缺失或非法时间返回 null（调用方据此不渲染时间段）。未来/时钟偏移回落「刚刚」。
 */
export function formatRelativeTime(iso: string | null | undefined, nowMs: number): string | null {
  if (!iso) return null;
  const t = new Date(iso).getTime();
  if (Number.isNaN(t)) return null;
  const diff = nowMs - t;
  if (diff < 60_000) return "刚刚";
  const minutes = Math.floor(diff / 60_000);
  if (minutes < 60) return `${minutes} 分钟前`;
  const hours = Math.floor(diff / 3_600_000);
  if (hours < 24) return `${hours} 小时前`;
  const days = Math.floor(diff / 86_400_000);
  if (days < 30) return `${days} 天前`;
  const months = Math.floor(days / 30);
  return `${months} 个月前`;
}

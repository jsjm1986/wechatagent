import type { StatusTone } from "../../components/ui/StatusBadge";

export function bucketTone(bucket: string): StatusTone {
  switch (bucket) {
    case "sent": return "running";
    case "pending": return "scheduled";
    case "blocked": return "blocked";
    case "escalated": return "held";
    default: return "inactive";
  }
}

export function bucketLabel(bucket: string): string {
  switch (bucket) {
    case "sent": return "已送达";
    case "pending": return "在途";
    case "blocked": return "被拦";
    case "escalated": return "已请示";
    case "canceled": return "已取消";
    case "skipped": return "去重跳过";
    default: return "未知";
  }
}

export function bucketCount(summary: Record<string, unknown>, bucket: string): number {
  const v = summary[bucket];
  if (typeof v === "number") return v;
  if (v && typeof v === "object") {
    return Object.values(v as Record<string, number>).reduce((a, b) => a + b, 0);
  }
  return 0;
}

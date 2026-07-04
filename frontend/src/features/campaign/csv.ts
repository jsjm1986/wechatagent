import type { CampaignSendItem } from "../../stores/campaignStore";
import { bucketLabel } from "./buckets";
import { SEND_OUTCOME_REASON_LABELS, labelOf } from "../../lib/reviewLabels";

function esc(v: string): string {
  if (/[",\n]/.test(v)) return '"' + v.replace(/"/g, '""') + '"';
  return v;
}

export function toCsv(items: CampaignSendItem[]): string {
  const header = "客户名,wxid,状态,原因";
  const rows = items.map((it) =>
    [esc(it.name || ""), esc(it.contactWxid), esc(bucketLabel(it.status)), esc(it.reason ? labelOf(SEND_OUTCOME_REASON_LABELS, it.reason) : "")].join(","),
  );
  return [header, ...rows].join("\r\n");
}

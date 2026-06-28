import type { CampaignSendItem } from "../../stores/campaignStore";
import { bucketLabel } from "./buckets";

function esc(v: string): string {
  if (/[",\n]/.test(v)) return '"' + v.replace(/"/g, '""') + '"';
  return v;
}

export function toCsv(items: CampaignSendItem[]): string {
  const header = "客户名,wxid,状态,原因";
  const rows = items.map((it) =>
    [esc(it.name || ""), esc(it.contactWxid), esc(bucketLabel(it.status)), esc(it.reason || "")].join(","),
  );
  return [header, ...rows].join("\r\n");
}

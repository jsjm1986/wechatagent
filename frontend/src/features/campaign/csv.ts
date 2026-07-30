import type { CampaignSendItem } from "../../stores/campaignStore";
import { bucketLabel } from "./buckets";
import { SEND_OUTCOME_REASON_LABELS, labelOf } from "../../lib/reviewLabels";

export function safeCsvCell(value: string): string {
  // Replace non-printing controls (tabs included) while preserving legitimate
  // multiline text. Spreadsheet quoting does not disable formulas, so prefix
  // every cell whose first non-whitespace character is a formula trigger.
  const normalized = value.replace(/[\u0000-\u0009\u000B\u000C\u000E-\u001F\u007F]/g, " ");
  const safe = /^\s*[=+\-@]/u.test(normalized) ? `'${normalized}` : normalized;
  if (/[",\n\r]/.test(safe)) return '"' + safe.replace(/"/g, '""') + '"';
  return safe;
}

export function toCsv(items: CampaignSendItem[]): string {
  const header = "客户名,wxid,状态,原因";
  const rows = items.map((it) =>
    [safeCsvCell(it.name || ""), safeCsvCell(it.contactWxid), safeCsvCell(bucketLabel(it.status)), safeCsvCell(it.reason ? labelOf(SEND_OUTCOME_REASON_LABELS, it.reason) : "")].join(","),
  );
  return [header, ...rows].join("\r\n");
}

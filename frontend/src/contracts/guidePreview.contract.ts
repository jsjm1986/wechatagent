// 后端 guide_preview_json 投影下发的 canonical 键集（抄自 fixture，非手猜）。
export const CANONICAL_KEYS = [
  "accountId",
  "contactId",
  "contactWxid",
  "createdAt",
  "health",
  "healthScores",
  "id",
  "impactScope",
  "instruction",
  "mode",
  "readableChanges",
  "riskWarnings",
  "scopeReason",
  "status",
  "suggestedChanges",
  "summary",
  "updatedAt",
] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];

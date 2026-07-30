// 后端 guide_preview_json 投影下发的 canonical 键集（抄自 fixture，非手猜）。
export const CANONICAL_KEYS = [
  "accountId",
  "authoritativeChanges",
  "candidateHash",
  "contactId",
  "contactWxid",
  "createdAt",
  "health",
  "healthScores",
  "id",
  "impactScope",
  "instruction",
  "mode",
  "playbookAffectedContacts",
  "readableChanges",
  "riskWarnings",
  "requiresStrongConfirmation",
  "scopeReason",
  "status",
  "suggestedChanges",
  "summary",
  "updatedAt",
] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];

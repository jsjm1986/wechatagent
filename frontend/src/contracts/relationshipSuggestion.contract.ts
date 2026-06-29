// 后端 relationship_suggestion_json 投影下发的 canonical 顶层键集(抄自 fixture,非手猜)。
export const CANONICAL_KEYS = [
  "accountId",
  "confidence",
  "contactId",
  "evidence",
  "firstSeenAt",
  "id",
  "lastSeenAt",
  "occurrences",
  "reviewedAt",
  "reviewedBy",
  "status",
  "suggestedValue",
  "workspaceId",
] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];

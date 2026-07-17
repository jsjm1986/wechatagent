// 后端 taxonomy_candidate_json 投影下发的 canonical 顶层键集(抄自 fixture,非手猜)。
export const CANONICAL_KEYS = [
  "confidence",
  "evidence",
  "firstSeenAt",
  "id",
  "kind",
  "lastSeenAt",
  "occurrences",
  "rawValue",
  "reviewedAt",
  "reviewedBy",
  "scope",
  "status",
  "suggestedDisplayName",
  "workspaceId",
] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];

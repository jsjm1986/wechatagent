// 后端 memory_candidate_json 投影下发的 canonical 键集（抄自 fixture，非手猜）。
export const CANONICAL_KEYS = [
  "accountId",
  "candidates",
  "contactWxid",
  "createdAt",
  "id",
  "memoryWriteScore",
  "reason",
  "runId",
  "source",
  "status",
  "updatedAt",
  "workspaceId",
] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];

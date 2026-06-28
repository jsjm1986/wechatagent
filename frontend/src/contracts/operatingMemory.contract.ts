// 后端 operating_memory_json 投影下发的 canonical 键集（抄自 fixture，非手猜）。
export const CANONICAL_KEYS = [
  "accountId",
  "contactWxid",
  "id",
  "memoryCard",
  "memoryCardUpdatedAt",
  "memoryCardVersion",
  "nextAction",
  "productFit",
  "relationshipState",
  "updatedAt",
  "userUnderstanding",
  "workspaceId",
] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];

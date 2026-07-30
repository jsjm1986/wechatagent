// 后端 llm_call_log_json 投影下发的 canonical 键集（抄自 fixture，非手猜）。
export const CANONICAL_KEYS = [
  "accountId",
  "completionTokens",
  "contactWxid",
  "createdAt",
  "error",
  "id",
  "latencyMs",
  "model",
  "promptCacheHitTokens",
  "promptCacheMissTokens",
  "promptKey",
  "promptTokens",
  "runId",
  "runMode",
  "status",
  "totalTokens",
  "usageKnown",
  "workspaceId",
] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];

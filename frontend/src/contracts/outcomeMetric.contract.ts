// 后端 outcome_metric_json 投影下发的 canonical 键集（抄自 fixture，非手猜）。
export const CANONICAL_KEYS = [
  "accountId",
  "agentBlockRate",
  "aiHoldClearedRate",
  "conversationDepth",
  "createdAt",
  "dailyRunCount",
  "dailyRunTokenTotal",
  "date",
  "horizon",
  "id",
  "replyRate",
] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];

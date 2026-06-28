// 后端 behavior_signal_metric_json 投影下发的 canonical 键集（抄自 fixture，非手猜）。
export const CANONICAL_KEYS = [
  "date",
  "dedupeSkipped",
  "errors",
  "id",
  "lastSuccessAt",
  "persisted",
  "updatedAt",
  "workspaceId",
] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];

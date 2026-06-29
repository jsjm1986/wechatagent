// 后端 operation_state_policy_json 投影下发的 canonical 顶层键集(抄自 fixture,非手猜)。
export const CANONICAL_KEYS = [
  "allowed",
  "currentVersion",
  "domain",
  "forbidden",
  "id",
  "previousVersion",
  "recommendedPace",
  "seededBy",
  "stateKey",
  "status",
  "updatedAt",
  "version",
  "workspaceId",
] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];

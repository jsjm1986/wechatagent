// 后端 runtime_flag_json 投影下发的 canonical 顶层键集(抄自 fixture,非手猜)。
export const CANONICAL_KEYS = [
  "enabled",
  "rolloutPercent",
  "rolloutPercentRaw",
  "thresholdAutoReleaseEnabled",
  "updatedAt",
  "updatedBy",
  "workspaceId",
] as const;

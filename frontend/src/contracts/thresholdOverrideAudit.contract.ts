// 后端 threshold_override_audit_json 投影下发的 canonical 顶层键集(抄自 fixture,非手猜)。
export const CANONICAL_KEYS = [
  "action",
  "decidedAt",
  "decidedBy",
  "gateKey",
  "hitRateObserved",
  "id",
  "newValue",
  "previousValue",
  "significanceMetrics",
  "sourceProposalId",
] as const;

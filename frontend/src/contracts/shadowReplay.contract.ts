// 后端 shadow_replay_json 投影下发的 canonical 顶层键集(抄自 fixture,非手猜)。
export const CANONICAL_KEYS = [
  "failureReason",
  "finishedAt",
  "id",
  "new5gateHit",
  "newFinalReviewStatus",
  "newReviewRisks",
  "newSelfCritiqueAddressed",
  "newTokenCost",
  "original5gateHit",
  "originalFinalReviewStatus",
  "originalSelfCritiqueAddressed",
  "similarityToOriginalText",
  "sourceRunId",
  "startedAt",
  "status",
] as const;

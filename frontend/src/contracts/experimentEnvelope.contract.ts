// 后端 experiment_envelope_json 投影下发的 canonical 顶层键集(抄自 fixture,非手猜)。
export const CANONICAL_KEYS = [
  "accountId",
  "budgetUsedCalls",
  "budgetUsedTokens",
  "cohortPromptSize",
  "cohortThresholdSize",
  "experimentId",
  "finishedAt",
  "proposalsCount",
  "proposalsEligibleCount",
  "startedAt",
  "status",
  "updatedAt",
  "windowHours",
  "workspaceId",
] as const;

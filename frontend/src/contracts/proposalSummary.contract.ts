// 后端 proposal_summary_json 投影下发的 canonical 顶层键集(抄自 fixture,非手猜)。
export const CANONICAL_KEYS = [
  "createdAt",
  "currentValue",
  "evalReplaysCompleted",
  "evalReplaysFailed",
  "failureReason",
  "gateKey",
  "id",
  "kind",
  "proposedSection",
  "proposedTemplateKey",
  "proposedValue",
  "significancePassed",
  "status",
  "updatedAt",
] as const;

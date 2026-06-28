// 后端 agent_run_json 投影下发的 canonical 键集（抄自 fixture，非手猜）。
export const CANONICAL_KEYS = [
  "accountId",
  "contactWxid",
  "context",
  "createdAt",
  "decision",
  "error",
  "gatewayResult",
  "id",
  "knowledgeRoute",
  "planner",
  "review",
  "runId",
  "status",
  "triggerKind",
  "workspaceId",
] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];

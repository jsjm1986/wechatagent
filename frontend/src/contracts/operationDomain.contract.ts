// 后端 operation_domain_json 投影下发的 canonical 顶层键集(抄自 fixture,非手猜)。
// 注:`runtimeParameters`/`stateMachine`/`askHumanPolicy` 等为嵌套对象,各算一个顶层键,不展开内部子键。
export const CANONICAL_KEYS = [
  "askHumanPolicy",
  "assistModeEnabled",
  "automationPolicy",
  "currentVersion",
  "domain",
  "goal",
  "id",
  "methodology",
  "name",
  "previousVersion",
  "reviewPolicy",
  "runtimeParameters",
  "seededBy",
  "stateMachine",
  "status",
  "toolPolicy",
  "updatedAt",
  "version",
  "workflow",
  "workspaceId",
] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];

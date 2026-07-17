// 后端 taxonomy_entry_json 投影下发的 canonical 顶层键集(抄自 fixture,非手猜)。
// 注:`value` 为嵌套对象,仅算一个顶层键,不展开内部子键。
export const CANONICAL_KEYS = [
  "currentVersion",
  "id",
  "kind",
  "previousVersion",
  "scope",
  "seededBy",
  "updatedAt",
  "value",
  "version",
  "workspaceId",
] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];

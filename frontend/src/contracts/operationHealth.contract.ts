// 后端 operation_health 聚合投影下发的顶层 canonical 键集（抄自 fixture，非手猜）。
// 对账只锁顶层 2 键；items/scores 内部形状由后端快照固定。
export const CANONICAL_KEYS = ["items", "scores"] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];

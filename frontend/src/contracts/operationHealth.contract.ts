// 后端 operation_health 聚合投影下发的顶层 canonical 键集（抄自 fixture，非手猜）。
// 对账锁顶层键；items/scores 内部形状由后端快照固定。
// inQuietHours/nextWakeAt/quietHoursEnabled 为驾驶舱作息灯只读字段（Task 1）。
export const CANONICAL_KEYS = [
  "items",
  "scores",
  "inQuietHours",
  "nextWakeAt",
  "quietHoursEnabled",
] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];

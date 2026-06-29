// 后端 experiment_summary_json 聚合投影下发的 canonical 顶层键集(抄自 fixture,非手猜)。
// experiment/proposals/proposalsCounts 为嵌套对象/数组,此处只列顶层键不展开。
export const CANONICAL_KEYS = [
  "experiment",
  "proposals",
  "proposalsCounts",
] as const;

// 后端 evaluation_scenario_json 投影下发的 canonical 顶层键集(抄自 fixture,非手猜;
// contactSeed/groundTruth 是嵌套对象,只列顶层键,不展开)。
export const CANONICAL_KEYS = [
  "accountId",
  "contactSeed",
  "createdAt",
  "description",
  "groundTruth",
  "id",
  "inboundMessages",
  "scenarioId",
  "status",
  "tags",
  "title",
  "updatedAt",
] as const;

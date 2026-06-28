// 契约对账声明 —— 知识用量日志投影 knowledge_usage_json 的线上键集。
// 仅服务契约对账测试,非业务类型。routeResult/toolTrace 经 into_relaxed_extjson 桥接为纯 JSON。
// 后端改投影→re-bless fixture→此处对账测红,强制前端同步。
export const CANONICAL_KEYS = [
  "id",
  "workspaceId",
  "accountId",
  "contactWxid",
  "runId",
  "knowledgeIds",
  "routeResult",
  "replyText",
  "reviewApproved",
  "blockedReason",
  "toolTrace",
  "createdAt",
] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];

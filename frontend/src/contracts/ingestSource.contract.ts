// 契约对账声明 —— 摄取源列表投影 ingest_source_json 的线上键集。
// 仅服务契约对账测试，非业务类型。list-ingest-sources 端点按此读源列表、
// 删除/重新激活按 sourceId 定位、间隔列读 scheduleMinutes。
// 后端改投影→re-bless fixture→此处对账测红，强制前端同步。
export const CANONICAL_KEYS = [
  "sourceId",
  "workspaceId",
  "kind",
  "url",
  "label",
  "scheduleMinutes",
  "lastFetchedAt",
  "lastEtag",
  "status",
  "failureStreak",
  "lastError",
  "ingestCount",
  "createdAt",
  "updatedAt",
] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];

// 契约对账声明 —— GET document 列表端点单条投影 operation_knowledge_document_json 的线上键集。
// 仅服务契约对账测试,非业务类型。后端改投影→re-bless fixture→此处对账测红,强制前端同步。
export const CANONICAL_KEYS = [
  "id",
  "workspaceId",
  "accountId",
  "domain",
  "sourceType",
  "sourceName",
  "title",
  "summary",
  "catalogSummary",
  "routingMap",
  "riskNotes",
  "rawContent",
  "contentHash",
  "lineIndex",
  "sectionIndex",
  "status",
  "version",
  "updatedAt",
] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];

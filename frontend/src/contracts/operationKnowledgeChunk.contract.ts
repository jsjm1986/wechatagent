// 契约对账声明 —— 列表端点 GET /api/operation-knowledge/chunks 单条投影
// (后端 operation_knowledge_chunk_json) 的"线上键集"。
//
// 这不是给视图用的业务类型(那是 types/index.ts);它只服务于契约对账测试:
// 前端在此显式声明"我知道后端会下发这 33 个键",vitest 把它与后端写出的
// fixture(operation_knowledge_chunk.fixture.json,前后端唯一真相源)做集合比对。
//
// 后端加了字段 → re-bless fixture → 前端对账测红(缺声明)→ 强制前端来此登记并处理。
// 后端删了字段 → re-bless fixture → 前端对账测红(声明多余死键)→ 强制前端清理。
// 这就是"后端更新后前端必须对应处理"的强制点。

/// 必有键:`json!` 宏对这些键无条件下发(Option 只把值变 null,键恒在)。
export const CANONICAL_KEYS = [
  "id",
  "workspaceId",
  "accountId",
  "documentId",
  "itemId",
  "domain",
  "knowledgeType",
  "businessContext",
  "title",
  "summary",
  "body",
  "applicableScenes",
  "notApplicableScenes",
  "sourceQuote",
  "sourceAnchors",
  "integrityStatus",
  "confidenceScore",
  "status",
  "priority",
  "wikiType",
  "chunkType",
  "dynamicConfidence",
  "usageStats",
  "lockedFields",
  "validFrom",
  "validTo",
  "relatedChunks",
  "businessTopics",
  "productTags",
  "supersededBy",
  "previousVersionId",
  "provenance",
  "updatedAt",
] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];

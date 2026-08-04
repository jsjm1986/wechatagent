// 契约对账声明 —— chunk 详情端点 get_operation_knowledge_chunk 的顶层线上键集。
// 详情与列表共用 operation_knowledge_chunk_json：item 内字段统一为 camelCase，
// ObjectId 为 hex 字符串，updatedAt 为 RFC3339。deep-link 审核因此可直接携带
// expectedUpdatedAt，不再解析 snake_case/BSON Extended JSON。内部字段由后端 fixture 固定；
// 本声明只锁顶层 item 包裹键。后端改形状后须 re-bless fixture 并跑契约测试。
export const CANONICAL_KEYS = ["item"] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];

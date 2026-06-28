// 契约对账声明 —— chunk 详情端点 get_operation_knowledge_chunk(crud.rs:357) 的线上键集。
// 详情端点 json!({"item": item}) 裸序列化 model:顶层只有 item 包裹,item 内部是 snake_case +
// {$oid}/{$date} BSON ExtJSON 包装 —— 与列表投影 operation_knowledge_chunk_json 的 camelCase
// 形状**冲突**(spec §9 刻意暴露,非缺陷)。本声明只锁顶层 item 包裹键;item 内部裸 struct 形状
// 由后端快照固定,前端若直接消费详情端点须自行处理 snake_case+ExtJSON(当前前端走列表端点)。
// 后端改详情端点形状→re-bless fixture→此处对账测红。
export const CANONICAL_KEYS = ["item"] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];

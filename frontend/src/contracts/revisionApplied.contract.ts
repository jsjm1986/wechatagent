// 契约对账声明 —— chunk patch/revision 响应投影 revision_applied_to_json 的线上键集。
// 仅服务契约对账测试,非业务类型。ok 为投影硬编码常量 true。
// 后端改投影→re-bless fixture→此处对账测红,强制前端同步。
export const CANONICAL_KEYS = [
  "ok",
  "revisionId",
  "chunkId",
  "op",
  "beforeHash",
  "afterHash",
  "unchanged",
] as const;

export type CanonicalKey = (typeof CANONICAL_KEYS)[number];

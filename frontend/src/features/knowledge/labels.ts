// 知识库频道统一文案翻译层 —— 只翻译"机器枚举值"(status/integrity/wikiType/severity 等
// 后端数据字段),不碰组件内人写的业务句子(那些在各组件内联,改动会撞保护性测试)。
//
// 范式沿用 trustTypes.ts 的 CHUNK_TYPE_LABELS:每个枚举一个 Record + xxxLabel(v?) 兜底,
// 未知值回退原文,避免枚举漂移时 UI 崩。chunkTypeLabel 从 trustTypes re-export 保持单一入口。

export { chunkTypeLabel, CHUNK_TYPE_LABELS } from "./trustTypes";

/// chunk 生命周期状态
export const STATUS_LABELS: Record<string, string> = {
  draft: "草稿",
  active: "在用",
  archived: "已归档",
};
export function statusLabel(v?: string | null): string {
  if (!v) return "—";
  return STATUS_LABELS[v] ?? v;
}

/// 完整性/审核状态(红线:AI 永不自动 verified,运营人工确认)
export const INTEGRITY_STATUS_LABELS: Record<string, string> = {
  needs_review: "待确认",
  verified: "已确认",
  rejected: "已退回",
  pending_verification: "待核验",
};
export function integrityStatusLabel(v?: string | null): string {
  if (!v) return "—";
  return INTEGRITY_STATUS_LABELS[v] ?? v;
}

/// 知识类型(它是什么知识,与 chunk_type"怎么用"正交)
export const WIKI_TYPE_LABELS: Record<string, string> = {
  source: "原始资料",
  entity: "实体",
  concept: "概念",
  comparison: "对比",
  synthesis: "综合",
  methodology: "方法论",
  finding: "结论",
  query: "查询",
  thesis: "命题",
  unknown: "未分类",
};
export function wikiTypeLabel(v?: string | null): string {
  if (!v) return "—";
  return WIKI_TYPE_LABELS[v] ?? v;
}

/// digest 卡片严重度(后端封闭枚举仅 info|warn|critical,fatal 已被后端过滤)
export const SEVERITY_LABELS: Record<string, string> = {
  info: "提示",
  warn: "注意",
  critical: "严重",
};
export function severityLabel(v?: string | null): string {
  if (!v) return "—";
  return SEVERITY_LABELS[v] ?? v;
}

/// 待评审分类
export const REVIEW_CATEGORY_LABELS: Record<string, string> = {
  contested: "有争议",
  needs_review: "待确认",
  source_orphan: "缺来源",
  pending_verification: "待核验",
  dependents_pending: "依赖待定",
};
export function reviewCategoryLabel(v?: string | null): string {
  if (!v) return "—";
  return REVIEW_CATEGORY_LABELS[v] ?? v;
}

/// 文档来源类型
export const SOURCE_TYPE_LABELS: Record<string, string> = {
  manual: "手动录入",
  imported_markdown: "文件导入",
  external_url: "外部网址",
  archived: "已归档",
};
export function sourceTypeLabel(v?: string | null): string {
  if (!v) return "—";
  return SOURCE_TYPE_LABELS[v] ?? v;
}

/// 收件箱待办优先级
export const PRIORITY_LABELS: Record<string, string> = {
  high: "高",
  mid: "中",
  low: "低",
};
export function priorityLabel(v?: string | null): string {
  if (!v) return "—";
  return PRIORITY_LABELS[v] ?? v;
}

/// 收件箱待办来源
export const ORIGIN_LABELS: Record<string, string> = {
  gap_signal: "知识缺口",
  digest: "今日要点",
  manual: "手动创建",
  lint: "质量信号",
};
export function originLabel(v?: string | null): string {
  if (!v) return "—";
  return ORIGIN_LABELS[v] ?? v;
}

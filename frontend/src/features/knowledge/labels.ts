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

/// 外部源抓取类型
export const SOURCE_KIND_LABELS: Record<string, string> = {
  rss: "RSS 订阅",
  html: "网页",
};
export function sourceKindLabel(v?: string | null): string {
  if (!v) return "—";
  return SOURCE_KIND_LABELS[v] ?? v;
}

/// 外部源运行状态
export const INGEST_STATUS_LABELS: Record<string, string> = {
  active: "正常",
  failing: "连续失败",
  disabled: "已停用",
  paused: "已暂停",
};
export function ingestStatusLabel(v?: string | null): string {
  if (!v) return "—";
  return INGEST_STATUS_LABELS[v] ?? v;
}

/// 风险等级(知识路由/试召的 riskLevel)
export const RISK_LEVEL_LABELS: Record<string, string> = {
  low: "低风险",
  medium: "中风险",
  high: "高风险",
  critical: "极高风险",
};
export function riskLevelLabel(v?: string | null): string {
  if (!v) return "—";
  return RISK_LEVEL_LABELS[v] ?? v;
}

/// 修订操作类型(chunk_revisions.op 封闭枚举)
export const REVISION_OP_LABELS: Record<string, string> = {
  create: "新建",
  patch: "修改",
  split: "拆分",
  merge: "合并",
  rollback: "回滚",
  archive: "归档",
  restore: "恢复",
  verify: "确认",
  unverify: "撤销确认",
};
export function revisionOpLabel(v?: string | null): string {
  if (!v) return "—";
  return REVISION_OP_LABELS[v] ?? v;
}

/// 修订来源(chunk_revisions.source 封闭枚举 ai|human|rule|imported)
export const REVISION_SOURCE_LABELS: Record<string, string> = {
  ai: "AI",
  human: "管理员",
  rule: "规则",
  imported: "导入",
};
export function revisionSourceLabel(v?: string | null): string {
  if (!v) return "—";
  return REVISION_SOURCE_LABELS[v] ?? v;
}

/// AI 协作草稿类型(chat 起草产物 draftKind)
export const DRAFT_KIND_LABELS: Record<string, string> = {
  chunk: "新增知识",
  chunk_update: "更新知识",
  pack_update: "更新话术包",
  digest_dispatch: "派发要点",
  operator_memory: "运营记忆",
};
export function draftKindLabel(v?: string | null): string {
  if (!v) return "—";
  return DRAFT_KIND_LABELS[v] ?? v;
}

/// 后台任务状态(knowledge_chat_tasks 封闭枚举)
export const TASK_STATUS_LABELS: Record<string, string> = {
  pending: "排队中",
  running: "执行中",
  completed: "已完成",
  failed: "失败",
  cancelled: "已取消",
};
export function taskStatusLabel(v?: string | null): string {
  if (!v) return "—";
  return TASK_STATUS_LABELS[v] ?? v;
}

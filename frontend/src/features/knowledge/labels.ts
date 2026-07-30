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

/// 完整性/审核状态(红线:AI 永不自动 verified,须由管理员确认)
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

/// 今日 Digest 报告状态
export const REPORT_STATUS_LABELS: Record<string, string> = {
  ok: "已生成",
  partial: "部分生成",
  running: "生成中",
  active: "已生成",
  generating: "生成中",
  failed: "生成失败",
  empty: "无内容",
};
export function reportStatusLabel(v?: string | null): string {
  if (!v) return "—";
  return REPORT_STATUS_LABELS[v] ?? v;
}

/// 今日 Digest 卡片类型
export const DIGEST_CARD_KIND_LABELS: Record<string, string> = {
  gap_coverage: "知识缺口",
  gap_signal: "知识缺口",
  stale_source: "来源过期",
  contested: "存在争议",
  needs_review: "待确认",
  quality_lint: "质量信号",
  ingest_failure: "抓取失败",
  follow_up: "跟进事项",
};
export function digestCardKindLabel(v?: string | null): string {
  if (!v) return "—";
  return DIGEST_CARD_KIND_LABELS[v] ?? v;
}

/// 今日 Digest 卡片建议动作(后端 knowledge_digest/mod.rs allowed_actions 封闭枚举)
export const DIGEST_SUGGESTED_ACTION_LABELS: Record<string, string> = {
  fix_chunk: "补字段/修复",
  add_chunk: "补录条目",
  retag: "重打标签",
  review_evolution: "评估进化提案",
  dismiss: "忽略",
  freeform: "查看详情",
};
export function digestSuggestedActionLabel(v?: string | null): string {
  if (!v) return "—";
  return DIGEST_SUGGESTED_ACTION_LABELS[v] ?? v;
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

/// 行业 Schema 字段类型(DomainField.kind 封闭枚举 string|enum|number|date|reference)
export const FIELD_KIND_LABELS: Record<string, string> = {
  string: "文本",
  enum: "固定选项",
  number: "数字",
  date: "日期",
  reference: "关联条目",
};
export function fieldKindLabel(v?: string | null): string {
  if (!v) return "—";
  return FIELD_KIND_LABELS[v] ?? v;
}

/// chunk 关系类型(models.rs:1488 六值闭集 superseded_by/references/requires/
/// contradicts/clarifies/refines;权威中文与关系新建下拉 shared.tsx 同源);未知回落原值。
export const RELATED_KIND_LABELS: Record<string, string> = {
  supports: "支持",
  contradicts: "矛盾",
  superseded_by: "被取代",
  references: "引用",
  requires: "依赖",
  clarifies: "澄清",
  refines: "细化",
};
export function relatedKindLabel(v?: string | null): string {
  if (!v) return "—";
  return RELATED_KIND_LABELS[v] ?? v;
}

/// AI 协作工坊单轮意图(chat.rs:1332 闭集);未知回落原值。
export const CHAT_INTENT_LABELS: Record<string, string> = {
  create_chunk: "起草知识",
  update_chunk: "修订知识",
  clarify_chunk: "澄清核对",
  digest_action: "摘要派工",
  update_operator_memory: "更新运营记忆",
  revoke_operator_memory: "撤销运营记忆",
  freeform: "自由对话",
};
export function chatIntentLabel(v?: string | null): string {
  if (!v) return "—";
  return CHAT_INTENT_LABELS[v] ?? v;
}

/// 运营记忆类型(memory.rs:2067 闭集 preference/rejection/context;
/// 中文与后端 chat.rs:959 同源);未知回落原值。
export const OPERATOR_MEMORY_KIND_LABELS: Record<string, string> = {
  preference: "偏好",
  rejection: "红线",
  context: "背景",
};
export function operatorMemoryKindLabel(v?: string | null): string {
  if (!v) return "—";
  return OPERATOR_MEMORY_KIND_LABELS[v] ?? v;
}

/// chunk 字段名 → 大白话中文名(AI 修复面板/字段锁展示用)。键形态可能 camelCase
/// 或 snake_case(来自 LLM 工具产物),两种都归一;措辞与 ReviewChat 的
/// PATCH_FIELD_LABELS/LOCKED_FIELD_LABELS 同源。未知字段回落原名,不吞。
export const CHUNK_FIELD_LABELS: Record<string, string> = {
  title: "标题",
  summary: "摘要",
  body: "正文",
  tags: "标签",
  priority: "优先级",
  sourceQuote: "原话出处",
  source_quote: "原话出处",
  sourceAnchors: "来源锚点",
  source_anchors: "来源锚点",
  knowledgeType: "知识类型",
  knowledge_type: "知识类型",
  chunkType: "知识类型",
  chunk_type: "知识类型",
};
export function chunkFieldLabel(v?: string | null): string {
  if (!v) return "—";
  return CHUNK_FIELD_LABELS[v] ?? v;
}

// ── 诊断仪表专用字典（PhaseRollup / WorkerHealth）──────────────────────────
// 全部取值经后端聚合写入点逐点亲验枚举；均带原值兜底，闭集外新值回落原样不崩。

/// run 终态(agent_run_logs.lifecycle,run_envelope.rs 7 值闭集)
export const RUN_LIFECYCLE_LABELS: Record<string, string> = {
  started: "已启动",
  running: "运行中",
  completed: "已完成",
  failed_before_decision: "决策前失败",
  failed_after_decision: "决策后失败",
  aborted_by_budget: "预算耗尽中止",
  aborted_by_external_signal: "外部信号中止",
};
export function runLifecycleLabel(v?: string | null): string {
  if (!v) return "—";
  return RUN_LIFECYCLE_LABELS[v] ?? v;
}

/// 单轮改写原因(agent_run_logs.revision_reason,gateway/gates 写入点枚举;
/// revision_llm_error:<err> 为动态前缀,按前缀归一)
export const REVISION_REASON_LABELS: Record<string, string> = {
  revision_applied_approved: "改写后二审通过",
  revisionDirection_empty: "改写方向为空跳过",
  budget_exceeded_before_revision: "改写前预算超额跳过",
  revision_post_review_failed: "改写后二审未过",
  revision_llm_timeout_30s: "改写模型 30 秒超时",
};
export function revisionReasonLabel(v?: string | null): string {
  if (!v) return "—";
  if (v.startsWith("revision_llm_error")) return "改写模型报错";
  return REVISION_REASON_LABELS[v] ?? v;
}

/// 审核员误判信号(agent_decision_reviews.reviewer_misjudge_signal,单值闭集)
export const REVIEWER_MISJUDGE_KIND_LABELS: Record<string, string> = {
  approved_but_user_negative: "放行后客户负反馈",
};
export function reviewerMisjudgeKindLabel(v?: string | null): string {
  if (!v) return "—";
  return REVIEWER_MISJUDGE_KIND_LABELS[v] ?? v;
}

/// 知识缺口信号状态(knowledge_gap_signals.status,observability.rs 5 值闭集)
export const GAP_SIGNAL_STATUS_LABELS: Record<string, string> = {
  pending: "待处理",
  auto_resolved: "规则自动消解",
  llm_resolved: "模型判定消解",
  applied: "已处理",
  dismissed: "已忽略",
};
export function gapSignalStatusLabel(v?: string | null): string {
  if (!v) return "—";
  return GAP_SIGNAL_STATUS_LABELS[v] ?? v;
}

/// 经验沉淀模式(lessons_learned.pattern_kind,3 值闭集)
export const LESSON_PATTERN_LABELS: Record<string, string> = {
  success: "成功案例",
  reviewer_misjudge_negative: "审核员误判负反馈",
  blocked_by_safety_guard: "安全门拦截",
};
export function lessonPatternLabel(v?: string | null): string {
  if (!v) return "—";
  return LESSON_PATTERN_LABELS[v] ?? v;
}

/// 知识对话任务失败错误类型(knowledge 任务/日报 error_kind,自由字符串,已知值兜底)
export const CHAT_ERROR_KIND_LABELS: Record<string, string> = {
  budget_exceeded: "预算耗尽",
  internal: "内部错误",
};
export function chatErrorKindLabel(v?: string | null): string {
  if (!v) return "—";
  return CHAT_ERROR_KIND_LABELS[v] ?? v;
}

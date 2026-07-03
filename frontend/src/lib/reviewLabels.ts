// C7/C8 共用:最终复核状态 + 暂缓类别的中文闭集标签。
// 闭集对齐后端:FINAL_REVIEW_STATUS_VALUES(run_envelope.rs:67-78,10 项)、
// HOLD_CATEGORY_VALUES(types.rs:1227-1231,3 项)。措辞遵守 AI 自主定位
// (见 scripts/ 下的自治禁词闸 CI lint,frontend/src 新增行受扫)。

export const FINAL_REVIEW_STATUS_LABELS: Record<string, string> = {
  approved: "已通过",
  revision_applied_approved: "改写后通过",
  revision_failed: "改写未达标",
  held_by_ai_policy: "AI 策略主动暂缓",
  blocked_by_safety_guard: "安全门拦截",
  ai_waiting_for_more_context: "AI 等待更多上下文",
  blocked_by_required_field: "必填信息缺失拦截",
  blocked_by_budget: "预算耗尽暂缓",
  blocked_unverified_product_claim: "未验证产品声明拦截",
  legacy_mode_unchecked: "兼容模式未校验",
};

export const HOLD_CATEGORY_LABELS: Record<string, string> = {
  held_by_ai_policy: "AI 策略主动暂缓",
  blocked_by_safety_guard: "安全门拦截",
  ai_waiting_for_more_context: "AI 等待更多上下文",
};

// gap_signal(sources_meta.rs:363 裸下发)。kind 10 类 gap_signals.rs 各判定点。
export const GAP_SIGNAL_KIND_LABELS: Record<string, string> = {
  orphan: "孤立知识",
  broken_link: "引用失效",
  missing_chunk: "依赖已归档",
  no_outlinks: "缺关联引用",
  low_confidence: "置信度偏低",
  stale: "时效已过",
  contradiction: "同题冲突",
  suggestion: "建议补完核实",
  dangling_anchor: "出处对不上",
  recall_miss: "知识缺口（答不上）",
};

export const GAP_SIGNAL_SEVERITY_LABELS: Record<string, string> = {
  info: "一般提示",
  warning: "需注意",
  error: "严重",
  high: "高优",
};

export const GAP_SIGNAL_STATUS_LABELS: Record<string, string> = {
  pending: "待处理",
  auto_resolved: "已自动消解",
  llm_resolved: "AI 已消解",
  applied: "已按建议处理",
  dismissed: "已忽略",
};

export const GAP_SIGNAL_SOURCE_LABELS: Record<string, string> = {
  rule: "规则检出",
  llm: "AI 判定",
  recall_trace: "对话追踪",
};

// escalation(principal_escalations.rs / ask_human_inbox.rs)。
export const ESCALATION_CATEGORY_LABELS: Record<string, string> = {
  high_risk_gated: "高风险待裁决",
  out_of_scope_decision: "超出职权待决策",
  stuck_or_undelivered: "多轮僵局待介入",
};

export const ESCALATION_VERDICT_LABELS: Record<string, string> = {
  approved: "同意",
  rejected: "拒绝",
  conditional: "有条件同意",
  deferred: "暂缓待定",
  delegated_back: "授权 AI 自行处理",
};

export const ESCALATION_RESOLVED_VIA_LABELS: Record<string, string> = {
  wechat: "领导微信裁决",
  admin: "后台裁决",
};

// 复核风险维度名(types.rs:1135-1160,含历史别名)。
export const RISK_DIMENSION_LABELS: Record<string, string> = {
  factRisk: "事实可靠度风险",
  hallucination_score: "事实可靠度风险",
  PressureRisk: "压迫感风险",
  pressure_risk: "压迫感风险",
  HumanLikeScore: "真人感评分",
  human_like: "真人感评分",
  EmotionalValue: "情绪价值评分",
  emotional_value: "情绪价值评分",
  ProductAccuracyScore: "产品准确度评分",
  knowledge_grounding_score: "产品准确度评分",
  boundary_privacy_safety: "边界隐私安全评分",
};

export function labelOf(
  map: Record<string, string>,
  value: string | null | undefined,
): string {
  if (value === null || value === undefined || value === "") return "—";
  return map[value] ?? value;
}

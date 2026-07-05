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

// gateway 过程态闭集(src/agent/run_envelope.rs GATEWAY_STATUS_VALUES,32 值)→ 中文。
// 单一真相源:先展开 FINAL_REVIEW_STATUS_LABELS(两闭集交集键复用同一措辞,消除口径漂移),
// 再补 gateway 独有的过程态键。default 回落原值(labelOf),未来新值不崩。
export const GATEWAY_STATUS_LABELS: Record<string, string> = {
  ...FINAL_REVIEW_STATUS_LABELS,
  pending: "待处理",
  allowed: "已放行",
  sent: "已发送",
  no_reply: "无需回复",
  review_blocked: "复核拦截",
  revision_skipped_invalid_direction: "改写跳过（方向无效）",
  revision_skipped_budget_exceeded: "改写跳过（预算超限）",
  revision_llm_failure: "改写时模型失败",
  tool_loop_timeout: "工具循环超时",
  not_managed: "未托管",
  cooldown: "冷却中",
  rate_limited: "限流中",
  daily_limit: "已达每日上限",
  expired: "已过期",
  context_changed: "上下文已变更",
  policy_cooldown: "策略冷却",
  policy_wait_user_reply: "等待客户回复",
  gateway_blocked: "网关拦截",
  precheck_blocked: "预检拦截",
  outbox_enqueued: "已入发件队列",
  admin_cancelled: "管理员已取消",
  superseded_by_new_inbound: "被更新消息取代",
  quiet_hours_deferred: "作息时段顺延",
};

// campaign 每人推送归桶原因(src/routes/campaigns.rs classify_send_outcome:388-440)。
// 主体是 GATEWAY_STATUS_VALUES 子集,复用 GATEWAY_STATUS_LABELS;另补 4 个 campaign 专有值:
// not_yet_run(task 未跑)/outbox 终态 failed_terminal、canceled/policy_consecutive_limit(不在 32 值闭集)。
export const SEND_OUTCOME_REASON_LABELS: Record<string, string> = {
  ...GATEWAY_STATUS_LABELS,
  not_yet_run: "尚未执行",
  failed_terminal: "发送失败（终态）",
  canceled: "已取消",
  policy_consecutive_limit: "连续触达限额",
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

// prompt / soul / prompt_template / evaluation_scenario 版本生命周期状态。
// 后端默认 draft(prompt_templates.rs:132)/ active(evaluation_scenario 默认)。
export const VERSION_STATUS_LABELS: Record<string, string> = {
  draft: "草稿",
  active: "生效中",
  published: "已发布",
  archived: "已归档",
};

// next_best_action.type：LLM 自由产出字段(types.rs:134 无闭集),已知 canonical 值
// reply / follow_up(guards.rs classify_decision_action:242/246);未知值回落原值。
export const NEXT_BEST_ACTION_TYPE_LABELS: Record<string, string> = {
  reply: "回复",
  follow_up: "主动跟进",
};

// review scores 维度 key → 中文。8 项已亲验(operations 现有 6 项 + replay.rs:358-362
// 的 factRisk / productAccuracy);未知维度经 labelOf 回落原 key 名,不吞。
export const REVIEW_SCORE_LABELS: Record<string, string> = {
  humanLike: "拟人度",
  emotionalValue: "情绪价值",
  hallucinationScore: "幻觉风险",
  knowledgeGroundingScore: "知识接地",
  pressureRisk: "压迫风险",
  boundaryPrivacySafety: "隐私边界",
  factRisk: "事实风险",
  productAccuracy: "产品准确度",
};

// prompt / soul / domain_profile / prompt_template 等版本化资源的写入来源
// (models.rs:1012/1044:system/manual/legacy_migration/evolution_release/generated_by_ai);未知回落原值。
export const SEEDED_BY_LABELS: Record<string, string> = {
  system: "系统内置",
  manual: "管理员新建",
  legacy_migration: "历史迁移",
  evolution_release: "自优化发布",
  generated_by_ai: "AI 生成",
};
export function seededByLabel(v?: string | null): string {
  if (!v) return "—";
  return SEEDED_BY_LABELS[v] ?? v;
}

// prompt 模板层级(与提示词新建下拉同源;后端 prompts.rs 5 值);未知回落原值。
export const PROMPT_LAYER_LABELS: Record<string, string> = {
  system_contract: "系统契约",
  policy: "运营规则",
  task_template: "任务模板",
  review: "复盘审查",
  methodology_generator: "方法论生成",
};
export function promptLayerLabel(v?: string | null): string {
  if (!v) return "—";
  return PROMPT_LAYER_LABELS[v] ?? v;
}

export function labelOf(
  map: Record<string, string>,
  value: string | null | undefined,
): string {
  if (value === null || value === undefined || value === "") return "—";
  return map[value] ?? value;
}

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

// 运营事件流 event.status（operations/index.tsx 时间线）→ 中文。
// status 主体是 gateway/precheck 状态族（复用 GATEWAY_STATUS_LABELS），再补
// 通用日志级状态。后端历史遗留三组同义拼写并存，两拼都收，否则漏显：
// warn/warning、success/succeeded、observe/observed。未知值经 labelOf 回落原值。
export const EVENT_STATUS_LABELS: Record<string, string> = {
  ...GATEWAY_STATUS_LABELS,
  ok: "正常",
  success: "成功",
  succeeded: "成功",
  info: "信息",
  warn: "提醒",
  warning: "提醒",
  error: "错误",
  failed: "失败",
  degraded: "降级",
  blocked: "已拦截",
  rejected: "已拒绝",
  deferred: "已延后",
  enqueued: "已入队",
  emitted: "已触达",
  skipped: "已跳过",
  skip: "跳过",
  capped: "达当日上限",
  recovered: "已恢复",
  retry: "重试中",
  dropped: "已丢弃",
  observed: "仅观测",
  observe: "仅观测",
  transitioned: "已流转",
  release: "已放量",
  partial: "部分完成",
  running: "执行中",
  completed: "已完成",
  cancelled: "已取消",
  finalize_review_blocked: "复核拦截",
};

// 运营事件流 event.kind → 中文。全量取值经逐写入点亲验枚举（webhook/gateway/
// outbox/planner/tasks/gates/knowledge/evolution 各写入路径，无 format! 动态拼接）。
// labelOf 带原值兜底：未来新增 kind 未收录时回落英文原样，不崩、不吞。
export const EVENT_KIND_LABELS: Record<string, string> = {
  // 入站 webhook
  webhook_unknown_app_id: "入站账号未注册",
  webhook_managed_contact_account_mismatch: "托管联系人账号不匹配",
  webhook_rate_limited: "入站触发限流",
  agent_error: "Agent 处理异常",
  webhook_handler_panic: "入站处理崩溃",
  quiet_hours_deferred_inbound: "作息时段入站顺延",
  // 发送网关 / 决策主链
  send_gateway_blocked: "后台发送被拦截",
  blocked_review: "发送复核拦截",
  management_send: "管理发送已入队",
  non_text_inbound_transition: "非文本入站过渡",
  state_action_policy_blocked: "状态策略拦截",
  agent_skipped: "Agent 跳过发送",
  ptier_self_assessment_malformed: "自评格式异常",
  ptier_forced_full: "强制升完整档",
  ptier_coverage_optimism: "知识覆盖偏乐观",
  ptier_relational_optimism: "关系维度偏乐观",
  ptier_escalated: "提示词升档",
  ptier_clarify: "触发澄清追问",
  ptier_run_tier: "本轮提示词档位",
  decision_phase_tool_calling_in_single_shot: "决策阶段工具调用降级",
  run_budget_exceeded: "本轮预算超额",
  style_consistency_revision_trigger: "风格一致性触发改写",
  revision_skipped_invalid_direction: "改写跳过（方向无效）",
  revision_skipped_budget_exceeded: "改写跳过（预算超限）",
  revision_llm_failure: "改写模型调用失败",
  gateway_blocked: "发送网关拦截",
  agent_reply: "Agent 已回复",
  outbox_enqueue_partial_failure: "发件入队部分失败",
  media_asset_id_invalid: "素材编号无效",
  media_asset_lookup_failed: "素材查询失败",
  media_asset_rejected: "素材被拒",
  media_asset_escalated: "素材升级审查",
  media_outbox_enqueue_failed: "素材发件入队失败",
  namecard_outbox_enqueue_failed: "名片发件入队失败",
  referral_card_rejected: "转介名片被拒",
  "agent.account_daily_send_soft_cap_exceeded": "账号当日发送超软上限",
  outbound_record_persist_failed: "出站记录落库失败",
  "agent.dimension_dropped": "画像维度被丢弃",
  "agent.stage_transition_rejected": "阶段流转被拒",
  "agent.commitment_field_missing": "承诺字段缺失",
  "agent.purchase_lifecycle_corrected_by_objective": "购买阶段被事实纠正",
  "agent.profile_churn_observed": "流失倾向观测",
  "agent.operation_state_transition_rejected": "运营状态流转被拒",
  "agent.operation_state_transitioned": "运营状态已流转",
  "agent.follow_up_run_at_degraded": "跟进时间计算降级",
  // 发件箱出站
  outbox_synthetic_idempotency_key: "合成幂等键",
  outbox_created: "发件已创建",
  outbox_idempotent_skip: "幂等命中跳过",
  outbox_canceled: "发件已取消",
  outbox_retry_scheduled: "发件重试已排期",
  outbox_failed_terminal: "发件最终失败",
  "agent.send_deferred_account_offline": "账号离线延后发送",
  "agent.send_deferred_account_pacing": "账号节流延后发送",
  outbox_sent_post_hoc: "发件事后确认",
  outbox_sent: "已发送",
  // 主动运营 planner
  strategic_planner_capped: "主动运营达当日上限",
  strategic_planner_emit: "主动唤醒触达",
  strategic_planner_tick: "主动运营巡检",
  strategic_planner_commitment_overdue: "承诺已到期提醒",
  strategic_planner_commitment_imminent: "承诺临近提醒",
  strategic_planner_commitment_tick: "承诺巡检",
  strategic_planner_stage_stagnation: "阶段停滞唤醒",
  strategic_planner_stage_stagnation_tick: "阶段停滞巡检",
  strategic_planner_calendar_care: "日历关怀触达",
  strategic_planner_calendar_tick: "日历关怀巡检",
  strategic_planner_renewal_reminder: "续费提醒",
  strategic_planner_renewal_tick: "续费巡检",
  strategic_planner_reactivation: "沉默激活触达",
  strategic_planner_reactivation_tick: "沉默激活巡检",
  strategic_planner_silent_backoff: "主动运营自主回退",
  strategic_planner_commitment_backoff: "承诺提醒自主回退",
  strategic_planner_stage_stagnation_backoff: "阶段停滞自主回退",
  strategic_planner_renewal_backoff: "续费提醒自主回退",
  strategic_planner_reactivation_backoff: "沉默激活自主回退",
  strategic_planner_backoff: "主动运营自主回退",
  // 后台任务 / worker
  claim_recovery_exhausted: "任务认领恢复耗尽",
  follow_up_failed: "跟进任务失败",
  task_claim_recovered: "任务认领已恢复",
  follow_up_processed: "跟进任务已处理",
  follow_up_retry_scheduled: "跟进重试已排期",
  cold_contact_emit: "冷联系人触达",
  cold_contact_tick: "冷联系人巡检",
  silence_signal_tick: "沉默信号巡检",
  background_worker_panic: "后台任务崩溃",
  account_scheduler_assignment: "账号调度分配",
  // 决策 finalize 拦截
  autonomy_field_violation: "自治必填字段违规",
  budget_exceeded_no_review: "预算超额且未复核",
  product_claim_blocked: "未验证产品声明拦截",
  grounding_probe_reviewer_missed: "接地探针发现漏判",
  autonomy_hold_category_invalid: "暂缓类别非法已矫正",
  // 记忆 / 知识 / 其它
  memory_conflict_resolved: "记忆冲突已解决",
  memory_consolidated: "记忆已固化",
  knowledge_unverified_warning: "命中未验证知识告警",
  run_envelope_recovered_via_insert: "运行记录已补写",
  knowledge_digest_generated: "知识日报已生成",
  knowledge_chat_task_finished: "知识长任务完成",
  knowledge_auto_verify_done: "知识自动校验完成",
  knowledge_chat_turn: "知识协作一轮",
  knowledge_chat_applied: "知识协作已应用",
  knowledge_operator_memory_added: "运营记忆已新增",
  knowledge_repair_proposed: "知识修复提案",
  knowledge_repair_applied: "知识修复已应用",
  "media_asset.reviewed": "素材已审核",
  "referral_card.reviewed": "名片已审核",
  outcome_event_marked: "成效事件已标记",
  user_operation_guide_applied: "运营指令已应用",
  taxonomy_version_changed: "分类字典版本变更",
  formula_adherence_evaluated: "公式遵守度已评测",
  lesson_promoted_to_peer_case: "经验已提升为同行案例",
  prompt_pack_reseed_fallback: "提示词包重种兜底",
  prompt_pack_align_skipped_evolution: "提示词对齐跳过（灰度中）",
  prompt_publish_kept_evolution_rows: "发布保留灰度历史",
  // 自优化 evolution
  evolution_budget_exceeded: "自优化预算超额",
  evolution_tick_completed: "自优化巡检完成",
  evolution_tick_failed: "自优化巡检失败",
  evolution_auto_release_decision: "自动放量决策",
  evolution_post_release_review: "放量后复盘",
  evolution_threshold_released: "阈值提案已放量",
  evolution_prompt_released: "提示词提案已放量",
  evolution_rollback_completed: "回滚完成",
};

export function labelOf(
  map: Record<string, string>,
  value: string | null | undefined,
): string {
  if (value === null || value === undefined || value === "") return "—";
  return map[value] ?? value;
}

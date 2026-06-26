// C7/C8 共用:最终复核状态 + 暂缓类别的中文闭集标签。
// 闭集对齐后端:FINAL_REVIEW_STATUS_VALUES(run_envelope.rs:67-78,10 项)、
// HOLD_CATEGORY_VALUES(types.rs:1227-1231,3 项)。措辞遵守 AI 自主定位
// (见 scripts/check-no-human-takeover.sh 的禁词闸,frontend/src 新增行受扫)。

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

export function labelOf(
  map: Record<string, string>,
  value: string | null | undefined,
): string {
  if (value === null || value === undefined || value === "") return "—";
  return map[value] ?? value;
}

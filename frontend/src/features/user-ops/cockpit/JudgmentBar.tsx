// 常驻判断条（Task 3）：把后端每轮 AI 判断顶到驾驶舱最前面。
// 6 chip：人格态 / 最近轮（finalReviewStatus 精确 10 态）/ 下一步 / 风险灯 / 作息灯 / 请示灯。
// 措辞遵守 AI 自主定位（frontend/src 新增行受 scripts/ 自治禁词闸扫）。
import { AlarmClockOff, AlertTriangle, Bot, MessageSquareReply, Route, Sparkles } from "lucide-react";
import type { Contact, DecisionReview, OperationHealth } from "../../../types";
import { FINAL_REVIEW_STATUS_LABELS, labelOf } from "../../../lib/reviewLabels";
import { labelFor, type TaxonomyMap } from "../../../stores/profileStore";
import { nextBestActionLabel } from "../legacy";
import styles from "./cockpit.module.css";

// finalReviewStatus（闭集 10 态，对齐 lib/reviewLabels.ts:6）→ 三色语义分组。
// 穷举 case 是闭集分组，非魔法值；未知/legacy/缺失回落 other（中性灰）。
export function finalReviewTone(status?: string): "sent" | "held" | "blocked" | "other" {
  switch (status) {
    case "approved":
    case "revision_applied_approved":
      return "sent";
    case "held_by_ai_policy":
    case "ai_waiting_for_more_context":
      return "held";
    case "blocked_by_safety_guard":
    case "blocked_by_required_field":
    case "blocked_by_budget":
    case "blocked_unverified_product_claim":
    case "revision_failed":
      return "blocked";
    default:
      return "other"; // legacy_mode_unchecked / 未知 / 缺失
  }
}

// tone → chip 底色 class（CSS Modules，色值全取 tokens.css）。
const TONE_CLASS: Record<"sent" | "held" | "blocked" | "other", string> = {
  sent: styles.chipSent,
  held: styles.chipHeld,
  blocked: styles.chipBlocked,
  other: styles.chipNeutral,
};

// 作息灯时间格式化：只取时:分，缺失/非法回落空串（调用方据此决定文案）。
function formatWakeTime(iso?: string | null): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

type JudgmentBarProps = {
  contact: Contact;
  latestReview?: DecisionReview;
  health: OperationHealth | null;
  escalationCount: number | null;
  taxonomies: TaxonomyMap;
  onRiskClick: () => void;
  onEscalationClick: () => void;
};

export function escalationCountLabel(count: number | null): string | null {
  if (count === null) return "请示计数不可用";
  return count > 0 ? `待决策请示 ${count}` : null;
}

export function JudgmentBar({
  contact,
  latestReview,
  health,
  escalationCount,
  taxonomies,
  onRiskClick,
  onEscalationClick,
}: JudgmentBarProps) {
  // 人格态：lastConversationMode 经字典翻译。设计预留键（后端当前不下发，
  // 见 types 里 Contact.lastConversationMode 注释），无值不渲染此 chip。
  const lastMode = contact.lastConversationMode || null;
  const modeLabel = lastMode ? labelFor(taxonomies, "conversation_mode", lastMode).text : null;

  // 最近轮：finalReviewStatus 精确 10 态映射（取代旧 approved?通过:拦截 二元）。
  const reviewTone = finalReviewTone(latestReview?.finalReviewStatus);
  const reviewLabel = latestReview
    ? labelOf(FINAL_REVIEW_STATUS_LABELS, latestReview.finalReviewStatus)
    : "尚无决策记录";

  // 下一步：复用 nextBestActionLabel（缺则回落）。
  const nextAction = nextBestActionLabel(latestReview?.nextBestAction);
  const nextActionText = nextAction && nextAction !== "-" ? nextAction : "等待用户消息";

  // 风险灯：健康度存在 danger 项即亮红，点击切观测段。
  const hasRisk = (health?.items || []).some((item) => item.tone === "danger");

  // 作息灯：Task 1 的 operation-health 只读顶层键（OperationHealth 已声明）；
  // inQuietHours 非 true（undefined/false）不渲染此 chip（优雅降级）。
  const inQuietHours = health?.inQuietHours === true;
  const wakeTime = formatWakeTime(health?.nextWakeAt);

  // 请示灯：正数可点击；null 显示不可用，不能和真实 0 混淆。
  const escalationLabel = escalationCountLabel(escalationCount);

  return (
    <div className={styles.judgmentBar} role="group" aria-label="AI 当前判断">
      {modeLabel && (
        <span className={`${styles.chip} ${styles.chipBrand}`}>
          <Bot size={13} />
          {modeLabel}
        </span>
      )}

      <span className={`${styles.chip} ${TONE_CLASS[reviewTone]}`}>
        <MessageSquareReply size={13} />
        {reviewLabel}
      </span>

      <span className={`${styles.chip} ${styles.chipNeutral}`}>
        <Route size={13} />
        下一步 · {nextActionText}
      </span>

      {hasRisk && (
        <button type="button" className={`${styles.chip} ${styles.chipBlocked} ${styles.chipButton}`} onClick={onRiskClick}>
          <AlertTriangle size={13} />
          运营健康风险
        </button>
      )}

      {inQuietHours && (
        <span className={`${styles.chip} ${styles.chipHeld}`}>
          <AlarmClockOff size={13} />
          {wakeTime
            ? `客户休息时段留言，将在 ${wakeTime} 后统一回复`
            : "客户休息时段留言，将在醒来时段统一回复"}
        </span>
      )}

      {escalationLabel &&
        (escalationCount === null ? (
          <span className={`${styles.chip} ${styles.chipHeld}`}>
            <Sparkles size={13} />
            {escalationLabel}
          </span>
        ) : (
          <button type="button" className={`${styles.chip} ${styles.chipScheduled} ${styles.chipButton}`} onClick={onEscalationClick}>
            <Sparkles size={13} />
            {escalationLabel}
          </button>
        ))}
    </div>
  );
}

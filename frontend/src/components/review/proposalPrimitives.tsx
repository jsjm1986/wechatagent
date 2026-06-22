// Ask-Human Phase 2 Task 6（零跨feature修订）：演化候选共享原语的中立家。
// 原本定义在 features/evolution/EvolutionCenterTab.tsx；为让 ProposalReleaseCard 与统一收件箱
// 频道复用而不反向依赖 features/evolution，逐字迁出到本中立模块（签名一字不改，
// 与既有 components/ui/StatusBadge / lib/format 的不同签名共存，保证老页渲染字节级不变）。
// 老页（EvolutionCenterTab）与卡片都从这里 import。
import styles from "./proposalPrimitives.module.css";
// 数值/百分比格式化收口到单一真相源 lib/format（审查#J）。
// formatNumber 直接复用（lib 版已支持可选 digits + NaN 守卫）；
// formatPercent 与 lib/format.formatRate 语义等价，按既有名薄封装 re-export，
// 保持 ProposalReleaseCard / EvolutionCenterTab 现有 import 名不破。
import { formatNumber, formatRate as formatPercent } from "../../lib/format";

export { formatNumber, formatPercent };

const STATUS_LABELS: Record<string, string> = {
  pending_eval: "待评测",
  evaluating: "评测中",
  eligible_for_release: "可发布",
  rejected_below_threshold: "未达标",
  released: "已发布",
  rolled_back: "已回滚",
};

const STATUS_TONES: Record<string, string> = {
  pending_eval: "neutral",
  evaluating: "info",
  eligible_for_release: "success",
  rejected_below_threshold: "warn",
  released: "primary",
  rolled_back: "danger",
};

// tone → CSS Module 徽章类（保留 data-tone 原值供测试断言；class 走局部化）。
const TONE_CLASS: Record<string, string> = {
  neutral: styles.badgeNeutral,
  info: styles.badgeInfo,
  success: styles.badgeSuccess,
  warn: styles.badgeWarn,
  primary: styles.badgePrimary,
  danger: styles.badgeDanger,
};

export function statusLabel(s: string): string {
  return STATUS_LABELS[s] ?? s;
}

export function statusTone(s: string): string {
  return STATUS_TONES[s] ?? "neutral";
}

export function StatusBadge({ status }: { status: string }) {
  const tone = statusTone(status);
  return (
    <span
      className={`${styles.badge} ${TONE_CLASS[tone] ?? styles.badgeNeutral}`}
      data-testid={`status-badge-${status}`}
      data-tone={tone}
    >
      {statusLabel(status)}
    </span>
  );
}

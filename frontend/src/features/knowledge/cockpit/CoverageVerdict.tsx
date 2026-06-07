import type { CompletenessView, CoverageState } from "../trustTypes";
import { StatusBadge, type StatusTone } from "../../../components/ui/StatusBadge";
import styles from "./CoverageVerdict.module.css";

interface CoverageVerdictProps {
  view: CompletenessView;
  onDrillDown: (dimKey: string) => void;
}

const TONE_BY_STATE: Record<CoverageState, StatusTone> = {
  verified: "running",
  draft: "held",
  missing: "blocked",
  methodology: "inactive",
};

const BADGE_TEXT: Record<CoverageState, string> = {
  verified: "可放心讲",
  draft: "待你审",
  missing: "空白·高风险",
  methodology: "只能讲思路",
};

function sayFor(state: CoverageState, dimKey: string): string {
  switch (state) {
    case "verified":
      return "有已验证的硬事实,AI 可以放心讲。";
    case "draft":
      return "有草稿还没核验,审过前 AI 不会用。";
    case "methodology":
      return "只有方法论/话术,能讲思路但不能给具体硬承诺。";
    case "missing":
      return dimKey === "effectClaims"
        ? "一条都没有。AI 一旦对客讲成功率/见效/回款,会被安全闸当场拦下。"
        : "这块还没有知识,AI 没法讲。";
  }
}

export function CoverageVerdict({ view, onDrillDown }: CoverageVerdictProps) {
  return (
    <div className={styles.vdList}>
      {view.dimensionList.map((dim) => (
        <button
          key={dim.key}
          type="button"
          className={styles.vd}
          onClick={() => onDrillDown(dim.key)}
        >
          <span className={styles.vdDim}>
            <span className={styles.vdLabel}>{dim.label}</span>
            <span className={styles.vdKey}>{dim.key}</span>
          </span>
          <span className={styles.vdBadge}>
            <StatusBadge tone={TONE_BY_STATE[dim.state]}>
              {BADGE_TEXT[dim.state]}
            </StatusBadge>
          </span>
          <span className={styles.vdSay}>{sayFor(dim.state, dim.key)}</span>
        </button>
      ))}
    </div>
  );
}

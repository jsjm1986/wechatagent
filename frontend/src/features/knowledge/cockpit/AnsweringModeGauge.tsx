import type { AnsweringMode } from "../trustTypes";
import styles from "./AnsweringModeGauge.module.css";

interface AnsweringModeGaugeProps {
  mode: AnsweringMode;
  needsReviewChunks: number;
  summary: string;
}

const MODE_MAP: Record<AnsweringMode, { label: string; level: number }> = {
  relationship_only: { label: "仅关系维护", level: 1 },
  product_safe: { label: "可安全讲产品", level: 2 },
  fully_supported: { label: "完全支撑", level: 3 },
};

export function AnsweringModeGauge({ mode, needsReviewChunks, summary }: AnsweringModeGaugeProps) {
  const { label, level } = MODE_MAP[mode];
  const fillPct = (level / 3) * 100;

  let reading = "";
  if (needsReviewChunks > 0 && mode !== "fully_supported") {
    reading = `距「完全支撑」差一步:有 ${needsReviewChunks} 条待审草稿,有待审草稿就绝不宣称完全支撑。审掉即解锁。`;
  } else if (needsReviewChunks === 0 && mode === "fully_supported") {
    reading = "知识库已完整支撑对客";
  } else {
    reading = summary;
  }

  return (
    <div className={styles.am}>
      <div className={styles.amRow}>
        <span className={styles.amDot} />
        <span className={styles.amVal}>{label}</span>
        <span className={styles.amCode}>answeringMode · {mode} · {level}/3 档</span>
      </div>
      <div className={styles.amMeter}>
        <div className={styles.amFill} style={{ width: `${fillPct}%` }} />
      </div>
      {reading && <div className={styles.amRead}>{reading}</div>}
    </div>
  );
}

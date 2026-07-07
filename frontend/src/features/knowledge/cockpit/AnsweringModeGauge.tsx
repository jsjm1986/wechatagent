import type { AnsweringMode, AnsweringModeLabels } from "../trustTypes";
import { DEFAULT_ANSWERING_MODE_LABELS } from "../trustTypes";
import styles from "./AnsweringModeGauge.module.css";

interface AnsweringModeGaugeProps {
  mode: AnsweringMode;
  needsReviewChunks: number;
  summary: string;
  // I：档位标签随 active DomainProfile 而来（非销售域换掉销售标签）。缺省回落内置销售标签。
  labels?: AnsweringModeLabels;
}

// 档位深度（1/2/3）是域无关的认知阶梯，恒定写死；档位中文标签由 profile 决定。
const MODE_LEVEL: Record<AnsweringMode, number> = {
  relationship_only: 1,
  product_safe: 2,
  fully_supported: 3,
};

export function AnsweringModeGauge({ mode, needsReviewChunks, summary, labels }: AnsweringModeGaugeProps) {
  const label = (labels ?? DEFAULT_ANSWERING_MODE_LABELS)[mode];
  const level = MODE_LEVEL[mode];
  const fillPct = (level / 3) * 100;

  let reading = "";
  if (needsReviewChunks > 0 && mode !== "fully_supported") {
    reading = `有 ${needsReviewChunks} 条待审草稿,只要还有草稿,就绝不宣称完全支撑。审掉草稿才有机会往上走(能不能到完全支撑,还看知识覆盖够不够全)。`;
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
        <span className={styles.amCode}>{level}/3 档</span>
      </div>
      <div className={styles.amMeter}>
        <div className={styles.amFill} style={{ width: `${fillPct}%` }} />
      </div>
      {reading && <div className={styles.amRead}>{reading}</div>}
    </div>
  );
}

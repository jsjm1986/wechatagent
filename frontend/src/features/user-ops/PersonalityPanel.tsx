import type { PersonalityProfile, PersonalityFacet } from "../../types";
import styles from "./PersonalityPanel.module.css";

/**
 * 标签可信度改造 - 大五 OCEAN 人格画像 + 演化图（子计划5 Task4）。
 *
 * 人格是 AI 从对话行为推断的评估层 → 紫色系（AI 身份，--color-brand）。
 * 五维各一条横向 bar（宽 = score 0~1 → 0~100%）；低置信维度（confidence < 0.3）
 * 灰化弱化 + 标"证据不足"——诚实原则：后端在无证据时强制 confidence=0，
 * UI 绝不为 0 置信度维度呈现一条"看起来很确信"的实色 bar。
 * 演化区复用 Task 3 手写 SVG 折线思路：snapshots>=2 时画五维五条线（x=快照序号）。
 */

// 五维 OCEAN → 中文名 + facet 取值键（顺序与后端 snapshot.scores 数组一致）。
const FACETS: Array<{ key: keyof Pick<PersonalityProfile,
  "openness" | "conscientiousness" | "extraversion" | "agreeableness" | "neuroticism">; label: string }> = [
  { key: "openness", label: "开放性" },
  { key: "conscientiousness", label: "尽责性" },
  { key: "extraversion", label: "外向性" },
  { key: "agreeableness", label: "宜人性" },
  { key: "neuroticism", label: "神经质" },
];

// 低置信阈值：低于此值视觉弱化 + 标"证据不足"。
const LOW_CONFIDENCE = 0.3;

// 演化折线画布几何（固定 viewBox），镜像 BayesianTrendChart。
const W = 320;
const H = 160;
const PAD_L = 28;
const PAD_R = 8;
const PAD_T = 10;
const PAD_B = 8;
const PLOT_W = W - PAD_L - PAD_R;
const PLOT_H = H - PAD_T - PAD_B;

// 局部紫系色板（tokens 仅单一 --color-brand，五线需可区分一组）。
const PALETTE = ["#5E5CE6", "#8B5CF6", "#A855F7", "#6366F1", "#7C3AED"];
const Y_TICKS = [0, 0.5, 1];

function clamp01(n: number): number {
  return Math.min(1, Math.max(0, n));
}

/** 把第 facetIdx 维在所有 snapshot 上的 score 映射成 polyline points。 */
function facetPoints(
  snapshots: PersonalityProfile["snapshots"],
  facetIdx: number
): string {
  const n = snapshots.length;
  return snapshots
    .map((s, i) => {
      const x = PAD_L + (n <= 1 ? 0 : (i / (n - 1)) * PLOT_W);
      const score = clamp01(s.scores?.[facetIdx] ?? 0);
      const y = PAD_T + (1 - score) * PLOT_H;
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
}

function FacetBar({ label, facet }: { label: string; facet: PersonalityFacet }) {
  const score = clamp01(facet.score);
  const conf = clamp01(facet.confidence);
  // 视觉弱化：置信度低于阈值即灰化（诚实——不呈现高确信外观）。
  const lowConf = conf < LOW_CONFIDENCE;
  // "证据不足"标签：后端无证据时强制 confidence=0，此时才贴该标签；
  // 低但非零置信仍如实展示百分比，不冒充"完全无据"。
  const noEvidence = conf <= 0;
  return (
    <div className={`${styles.facet} ${lowConf ? styles.lowConf : ""}`}>
      <div className={styles.facetHead}>
        <span className={styles.facetName}>{label}</span>
        {noEvidence ? (
          <span className={styles.lowConfTag}>证据不足</span>
        ) : (
          <span className={styles.facetConf}>置信度 {Math.round(conf * 100)}%</span>
        )}
      </div>
      <div className={styles.track} role="presentation">
        <div className={styles.fill} style={{ width: `${(score * 100).toFixed(0)}%` }} />
      </div>
    </div>
  );
}

export default function PersonalityPanel({ profile }: { profile?: PersonalityProfile }) {
  if (!profile) {
    return (
      <div className={styles.empty}>暂无人格分析（需多轮对话归并后推断）</div>
    );
  }

  const snapshots = profile.snapshots ?? [];

  return (
    <div className={styles.wrap}>
      <p className={styles.framing}>基于大五人格（OCEAN），从对话行为推断，仅供参考</p>

      <div className={styles.facets}>
        {FACETS.map((f) => (
          <FacetBar key={f.key} label={f.label} facet={profile[f.key]} />
        ))}
      </div>

      {/* 演化区：snapshots>=2 画五维折线，否则提示 */}
      <div className={styles.evolution}>
        <div className={styles.evoTitle}>人格演化</div>
        {snapshots.length >= 2 ? (
          <div className={styles.chartWrap}>
            <svg
              className={styles.svg}
              viewBox={`0 0 ${W} ${H}`}
              role="img"
              aria-label="人格演化折线图"
              preserveAspectRatio="none"
            >
              {Y_TICKS.map((t) => {
                const y = PAD_T + (1 - t) * PLOT_H;
                return (
                  <g key={t}>
                    <line className={styles.gridLine} x1={PAD_L} y1={y} x2={W - PAD_R} y2={y} />
                    <text className={styles.tickLabel} x={PAD_L - 5} y={y + 3} textAnchor="end">
                      {t}
                    </text>
                  </g>
                );
              })}
              {FACETS.map((f, idx) => (
                <polyline
                  key={f.key}
                  className={styles.line}
                  points={facetPoints(snapshots, idx)}
                  fill="none"
                  stroke={PALETTE[idx % PALETTE.length]}
                />
              ))}
            </svg>
            <ul className={styles.legend}>
              {FACETS.map((f, idx) => (
                <li key={f.key} className={styles.legendItem}>
                  <span
                    className={styles.swatch}
                    style={{ background: PALETTE[idx % PALETTE.length] }}
                    aria-hidden="true"
                  />
                  <span className={styles.legendName}>{f.label}</span>
                </li>
              ))}
            </ul>
          </div>
        ) : (
          <div className={styles.evoHint}>演化需多次归并后呈现</div>
        )}
      </div>
    </div>
  );
}

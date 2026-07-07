import type { BayesianSignal } from "../../types";
import { useProfileStore, labelFor } from "../../stores/profileStore";
import styles from "./BayesianTrendChart.module.css";

/**
 * 标签可信度改造 - 贝叶斯置信度走势图（子计划5 Task3）。
 * 手写 SVG 折线图（不引图表库）：每个已占槽（locked）维度一条 polyline，
 * x = 历史轮次（按 history 索引均匀分布），y = 置信度 0~1 映射到画布。
 * 贝叶斯是 AI 评估层 → 紫色系（AI 身份）；多线用局部紫系色板区分。
 * 未占槽（locked=false）维度不画线——未积累足够强证据，尚未赢得槽位。
 */

// 画布几何（固定 viewBox）。
const W = 320;
const H = 160;
const PAD_L = 28; // 左侧留 y 轴刻度标签
const PAD_R = 8;
const PAD_T = 10;
const PAD_B = 8;
const PLOT_W = W - PAD_L - PAD_R;
const PLOT_H = H - PAD_T - PAD_B;

// 局部紫系色板（tokens.css 仅有单一 --color-brand 紫，多线需可区分的一组）。
const PALETTE = ["#5E5CE6", "#8B5CF6", "#A855F7", "#6366F1", "#7C3AED", "#4F46E5"];

const Y_TICKS = [0, 0.5, 1];

/** 把一个 locked signal 的 history 映射成 polyline 的 points 字符串。 */
function toPoints(history: BayesianSignal["history"]): string {
  const n = history.length;
  return history
    .map((p, i) => {
      // x：按索引均匀分布；单点时落在左端。
      const x = PAD_L + (n <= 1 ? 0 : (i / (n - 1)) * PLOT_W);
      // y：confidence 0~1 → 顶部为 1、底部为 0。
      const conf = Math.min(1, Math.max(0, p.confidence));
      const y = PAD_T + (1 - conf) * PLOT_H;
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
}

export default function BayesianTrendChart({ signals }: { signals: BayesianSignal[] }) {
  const taxonomies = useProfileStore((s) => s.taxonomies);
  const dimensions = useProfileStore((s) => s.dimensions);
  const locked = signals.filter((s) => s.locked);
  // dimension 是维度 kind（如 customer_stage）→ 显示名；currentValue 是该维度取值 → 中文标签。
  const dimName = (kind: string) => dimensions.find((d) => d.kind === kind)?.displayName || kind;

  if (locked.length === 0) {
    return (
      <div className={styles.empty}>暂无评估维度（需多轮强证据才占槽）</div>
    );
  }

  return (
    <div className={styles.wrap}>
      <svg
        className={styles.svg}
        viewBox={`0 0 ${W} ${H}`}
        role="img"
        aria-label="贝叶斯置信度走势图"
        preserveAspectRatio="none"
      >
        {/* y 轴刻度线 0 / 0.5 / 1 */}
        {Y_TICKS.map((t) => {
          const y = PAD_T + (1 - t) * PLOT_H;
          return (
            <g key={t}>
              <line
                className={styles.gridLine}
                x1={PAD_L}
                y1={y}
                x2={W - PAD_R}
                y2={y}
              />
              <text className={styles.tickLabel} x={PAD_L - 5} y={y + 3} textAnchor="end">
                {t}
              </text>
            </g>
          );
        })}
        {/* 每个 locked 维度一条折线 */}
        {locked.map((s, idx) => (
          <polyline
            key={s.dimension}
            className={styles.line}
            points={toPoints(s.history)}
            fill="none"
            stroke={PALETTE[idx % PALETTE.length]}
          />
        ))}
      </svg>

      {/* 图例：维度名 + 当前值 + 当前置信度 */}
      <ul className={styles.legend}>
        {locked.map((s, idx) => (
          <li key={s.dimension} className={styles.legendItem}>
            <span
              className={styles.swatch}
              style={{ background: PALETTE[idx % PALETTE.length] }}
              aria-hidden="true"
            />
            <span className={styles.legendName}>{dimName(s.dimension)}</span>
            <span className={styles.legendValue}>
              {labelFor(taxonomies, s.dimension, s.currentValue).text} · 置信度 {Math.round(s.currentConfidence * 100)}%
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

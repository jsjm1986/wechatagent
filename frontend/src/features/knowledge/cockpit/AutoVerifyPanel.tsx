import { useState } from "react";
import { Sparkles } from "lucide-react";
import styles from "./AutoVerifyPanel.module.css";

interface AutoVerifyResult {
  processed: number;
  verified: number;
  needsReview: number;
  rejected: number;
  needsHumanAudit: number;
  degraded?: boolean;
}

type Tightness = "loose" | "medium" | "strict";
type Count = 50 | 100 | 500;

const THRESHOLD: Record<Tightness, number> = { loose: 5, medium: 7, strict: 9 };

const TIGHTNESS_OPTS: { key: Tightness; label: string }[] = [
  { key: "loose", label: "宽松" },
  { key: "medium", label: "适中" },
  { key: "strict", label: "严格" },
];

const TIGHTNESS_HINT: Record<Tightness, string> = {
  loose: "宽松= AI 稍微觉得行(≥5 分)就标通过，挑出来的多、但要你多复查",
  medium: "适中= AI 比较有把握(≥7 分)才标通过",
  strict: "严格= AI 很有把握(≥9 分)才标通过，挑出来的少、最稳妥",
};

const COUNT_OPTS: { key: Count; label: string }[] = [
  { key: 50, label: "最近 50 条" },
  { key: 100, label: "最近 100 条" },
  { key: 500, label: "全部待处理" },
];

export function AutoVerifyPanel(_props: { onClose?: () => void }) {
  void _props;
  const [tightness, setTightness] = useState<Tightness>("medium");
  const [keepReview, setKeepReview] = useState(true);
  const [count, setCount] = useState<Count>(50);
  const [result, setResult] = useState<AutoVerifyResult | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function start() {
    setRunning(true);
    setError(null);
    try {
      const r = await globalThis.fetch("/api/operation-knowledge/auto-verify", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          confidence_threshold: THRESHOLD[tightness],
          human_audit_sample_rate: keepReview ? 0.1 : 0,
          limit: count,
        }),
      });
      if (!r.ok) throw new Error("fail");
      const raw = (await r.json()) as Partial<AutoVerifyResult>;
      // 后端漏字段时兜 0,避免下方算出 NaN 渲染给用户
      setResult({
        processed: raw.processed ?? 0,
        verified: raw.verified ?? 0,
        needsReview: raw.needsReview ?? 0,
        rejected: raw.rejected ?? 0,
        needsHumanAudit: raw.needsHumanAudit ?? 0,
        degraded: raw.degraded,
      });
    } catch {
      setError("筛选没跑成功，稍后再试。");
    } finally {
      setRunning(false);
    }
  }

  const noConfidence = result ? result.needsReview + result.rejected : 0;

  return (
    <div className={styles.panel}>
      {/* 点题条：让运营明白「是我让 AI 帮我筛，不是 AI 替我做主」 */}
      <section className={styles.intro}>
        <div className={styles.introHead}>
          <Sparkles size={16} />
          <span className={styles.introTitle}>让 AI 帮你筛一遍待处理的知识</span>
        </div>
        <p className={styles.introBody}>
          你定个把关松紧，AI 把明显没问题的挑出来标好，可疑的先搁着，还随机抽一批请你亲自把关。
          <strong className={styles.introStrong}>AI 不会替你放行没把握的。</strong>
        </p>
      </section>

      {/* 三档松紧 */}
      <section className={styles.block}>
        <span className={styles.blockLabel}>把关松紧</span>
        <div className={styles.segGroup}>
          {TIGHTNESS_OPTS.map((opt) => (
            <button
              key={opt.key}
              type="button"
              className={tightness === opt.key ? `${styles.seg} ${styles.segOn}` : styles.seg}
              onClick={() => setTightness(opt.key)}
            >
              {opt.label}
            </button>
          ))}
        </div>
        <p className={styles.hint}>{TIGHTNESS_HINT[tightness]}</p>
      </section>

      {/* 留一批我复查 */}
      <section className={styles.block}>
        <label className={styles.toggleRow}>
          <input
            type="checkbox"
            className={styles.toggleInput}
            checked={keepReview}
            onChange={(e) => setKeepReview(e.target.checked)}
          />
          <span className={styles.toggleText}>
            <span className={styles.toggleTitle}>留一批我复查</span>
            <span className={styles.toggleSub}>
              即使 AI 标了通过，也随机留 10% 让我再看一眼（更保险）
            </span>
          </span>
        </label>
      </section>

      {/* 筛多少条 */}
      <section className={styles.block}>
        <span className={styles.blockLabel}>筛多少条</span>
        <div className={styles.segGroup}>
          {COUNT_OPTS.map((opt) => (
            <button
              key={opt.key}
              type="button"
              className={count === opt.key ? `${styles.seg} ${styles.segOn}` : styles.seg}
              onClick={() => setCount(opt.key)}
            >
              {opt.label}
            </button>
          ))}
        </div>
      </section>

      <button type="button" className={styles.startBtn} onClick={() => void start()} disabled={running}>
        <Sparkles size={15} />
        {running ? "筛选中…" : "开始筛"}
      </button>

      {error ? <p className={styles.error}>{error}</p> : null}

      {/* 结果三堆 */}
      {result ? (
        <section className={styles.block}>
          <span className={styles.blockLabel}>
            筛完了 · 共看了 {result.processed} 条
            {result.degraded ? "（有一批 AI 没能细看，已按保守处理）" : ""}
          </span>
          <div className={styles.pileGrid}>
            <div className={`${styles.pile} ${styles.pileVerified}`}>
              <span className={styles.pileNum}>{result.verified}</span>
              <span className={styles.pileTitle}>AI 觉得没问题</span>
              <span className={styles.pileSub}>可直接用，不放心也能逐条看</span>
            </div>
            <div className={`${styles.pile} ${styles.pileAudit}`}>
              <span className={styles.pileNum}>{result.needsHumanAudit}</span>
              <span className={styles.pileTitle}>留给你复查</span>
              <span className={styles.pileSub}>AI 虽觉得行，随机抽出请你把关</span>
            </div>
            <div className={`${styles.pile} ${styles.pileUnsure}`}>
              <span className={styles.pileNum}>{noConfidence}</span>
              <span className={styles.pileTitle}>AI 没把握，没动</span>
              <span className={styles.pileSub}>多半缺出处，还留在待处理</span>
            </div>
          </div>
        </section>
      ) : null}
    </div>
  );
}

import { useCallback, useEffect, useState } from "react";
import { ShieldCheck } from "lucide-react";
import { parseCompleteness, parseIntegrityReport, type CompletenessView, type IntegrityReportView } from "../trustTypes";
import { AnsweringModeGauge } from "./AnsweringModeGauge";
import { CoverageVerdict } from "./CoverageVerdict";
import { MetricCard } from "../../../components/ui/MetricCard/MetricCard";
import styles from "./CockpitView.module.css";

interface CockpitViewProps {
  onOpenReview: (dimKey?: string) => void;
  onOpenAutoVerify: () => void;
}

export function CockpitView({ onOpenReview, onOpenAutoVerify }: CockpitViewProps) {
  const [completeness, setCompleteness] = useState<CompletenessView | null>(null);
  const [integrity, setIntegrity] = useState<IntegrityReportView | null>(null);
  const [gapPendingCount, setGapPendingCount] = useState(0);
  const [loadFailed, setLoadFailed] = useState(false);

  const load = useCallback(() => {
    let alive = true;
    setLoadFailed(false);
    Promise.all([
      fetch("/api/operation-knowledge/completeness")
        .then((r) => (r.ok ? r.json() : null))
        .then((j) => (j ? parseCompleteness(j) : null))
        .catch(() => null),
      fetch("/api/operation-knowledge/integrity-report")
        .then((r) => (r.ok ? r.json() : null))
        .then((j) => (j ? parseIntegrityReport(j) : null))
        .catch(() => null),
      fetch("/api/knowledge/gap-signals?status=pending")
        .then((r) => (r.ok ? r.json() : null))
        .catch(() => null),
    ]).then(([comp, integ, gaps]) => {
      if (!alive) return;
      setCompleteness(comp);
      setIntegrity(integ);
      const signals = gaps && Array.isArray((gaps as { signals?: unknown[] }).signals)
        ? (gaps as { signals: unknown[] }).signals
        : [];
      setGapPendingCount(signals.length);
      if (!comp) setLoadFailed(true);
    });
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => load(), [load]);

  if (loadFailed) {
    return (
      <div className={styles.loading}>
        没读到知识库状态，可能是网络或服务没响应。
        <button type="button" className={styles.retry} onClick={() => load()}>
          重新加载
        </button>
      </div>
    );
  }

  if (!completeness) {
    return <div className={styles.loading}>正在加载知识库状态…</div>;
  }

  return (
    <div className={styles.cockpit}>
      <section className={styles.gaugeWrap}>
        <AnsweringModeGauge
          mode={completeness.answeringMode}
          needsReviewChunks={completeness.needsReviewChunks}
          summary={completeness.summary}
          labels={completeness.answeringModeLabels}
        />
      </section>

      <section className={styles.block}>
        <span className={styles.sectionLabel}>知识覆盖</span>
        <CoverageVerdict view={completeness} onDrillDown={onOpenReview} />
      </section>

      <section className={styles.block}>
        <span className={styles.sectionLabel}>治理待办</span>
        <div className={styles.todoGrid}>
          <MetricCard
            label="待审草稿"
            value={integrity?.needsReview ?? 0}
            detail="审过前 AI 不会用"
            onClick={() => onOpenReview()}
          />
          <MetricCard
            label="D2 降级"
            value={integrity?.anchorsMissing ?? 0}
            detail="active 但缺原文锚点"
            onClick={() => onOpenReview()}
          />
          <MetricCard
            label="知识缺口"
            value={gapPendingCount}
            detail="待处理的缺口信号"
            onClick={() => onOpenReview()}
          />
        </div>
        <button type="button" className={styles.autoVerify} onClick={onOpenAutoVerify}>
          <ShieldCheck size={15} />
          批量自动校验
        </button>
      </section>

      {completeness.gaps.length > 0 && (
        <section className={styles.block}>
          <span className={styles.sectionLabel}>缺口明细</span>
          <ul className={styles.gapList}>
            {completeness.gaps.map((gap, i) => (
              <li key={i} className={styles.gapItem}>
                <span className={styles.gapBullet} aria-hidden="true" />
                {gap}
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}

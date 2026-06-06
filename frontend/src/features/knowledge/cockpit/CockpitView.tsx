import { useEffect, useState } from "react";
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

  useEffect(() => {
    let alive = true;
    Promise.all([
      fetch("/api/operation-knowledge/completeness")
        .then((r) => (r.ok ? r.json() : null))
        .then((j) => (j ? parseCompleteness(j) : null))
        .catch(() => null),
      fetch("/api/operation-knowledge/integrity-report")
        .then((r) => (r.ok ? r.json() : null))
        .then((j) => (j ? parseIntegrityReport(j) : null))
        .catch(() => null),
    ]).then(([comp, integ]) => {
      if (!alive) return;
      setCompleteness(comp);
      setIntegrity(integ);
    });
    return () => {
      alive = false;
    };
  }, []);

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
            label="需复核"
            value={integrity?.rejected ?? 0}
            detail="已降级/驳回待处理"
            onClick={() => onOpenReview()}
          />
          <MetricCard
            label="知识总数"
            value={integrity?.total ?? 0}
            detail="知识库全部条目"
            onClick={() => onOpenReview()}
          />
        </div>
        <button type="button" className={styles.autoVerify} onClick={onOpenAutoVerify}>
          <ShieldCheck size={15} />
          批量自动校验 · auto-verify
        </button>
      </section>
    </div>
  );
}

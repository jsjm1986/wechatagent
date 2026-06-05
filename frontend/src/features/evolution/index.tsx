import { useEffect, useState } from "react";
import { ShieldCheck } from "lucide-react";
import { EvolutionCenterTab } from "./EvolutionCenterTab";
import styles from "./EvolutionCenterTab.module.css";

// 演化中心频道：自取 /api/health 判定演化器是否启用，再委托 EvolutionCenterTab。
// 大页头（eyebrow/title/subtitle）由 Shell 依据 channels.ts 渲染，组件仅保留面板级小标题。
export default function EvolutionFeature() {
  const [enabled, setEnabled] = useState<boolean | null>(null);

  useEffect(() => {
    let cancelled = false;
    fetch("/api/health")
      .then((r) => (r.ok ? r.json() : { evolutionEnabled: false }))
      .then((d: { evolutionEnabled?: boolean }) => {
        if (!cancelled) setEnabled(d.evolutionEnabled === true);
      })
      .catch(() => {
        if (!cancelled) setEnabled(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <div className={styles.page}>
      <section className={styles.panel}>
        <div className={styles.panelHead}>
          <div className={styles.panelHeadL}>
            <span className={styles.eyebrow}>Self Evolution</span>
            <span className={styles.title}>实验信封 · 候选 · Shadow 评测</span>
          </div>
          <div className={styles.headIcon}>
            <ShieldCheck size={18} />
          </div>
        </div>
        {enabled === null ? (
          <div className={styles.loading}>加载中…</div>
        ) : (
          <EvolutionCenterTab enabled={enabled} />
        )}
      </section>
    </div>
  );
}

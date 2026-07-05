import { ShieldCheck } from "lucide-react";
import { EvolutionCenterTab } from "./EvolutionCenterTab";
import styles from "./EvolutionCenterTab.module.css";

// 演化中心频道：直接委托 EvolutionCenterTab。运维硬锁定态（EVOLUTION_ENABLED=false）
// 与总开关态均由 Tab 内 loadFlag 拉取的 envEvolutionEnabled / flag 单数据源驱动。
// 大页头（eyebrow/title/subtitle）由 Shell 依据 channels.ts 渲染，组件仅保留面板级小标题。
export default function EvolutionFeature() {
  return (
    <div className={styles.page}>
      <section className={styles.panel}>
        <div className={styles.panelHead}>
          <div className={styles.panelHeadL}>
            <span className={styles.eyebrow}>Self Evolution</span>
            <span className={styles.title}>实验信封 · 候选 · 影子评测</span>
          </div>
          <div className={styles.headIcon}>
            <ShieldCheck size={18} />
          </div>
        </div>
        <EvolutionCenterTab />
      </section>
    </div>
  );
}

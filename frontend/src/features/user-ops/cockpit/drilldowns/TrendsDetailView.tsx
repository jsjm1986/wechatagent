// 走势下钻（Task 7，补 spec §5.1 item5）。观测段人格卡只放 OCEAN bar 缩略，
// 此下钻展开完整走势：人格演化折线（PersonalityPanel 内含 snapshots 折线）+
// 贝叶斯置信度走势（BayesianTrendChart）。数据全部取自 Contact，无新增 props。
import { ArrowLeft } from "lucide-react";
import type { Contact } from "../../../../types";
import PersonalityPanel from "../../PersonalityPanel";
import BayesianTrendChart from "../../BayesianTrendChart";
import styles from "../cockpit.module.css";

export function TrendsDetailView({ contact, onBack }: { contact: Contact; onBack: () => void }) {
  return (
    <section className="smartTabPanel">
      <div className={styles.drilldownHead}>
        <button className={styles.backButton} type="button" onClick={onBack}>
          <ArrowLeft size={15} />
          返回
        </button>
        <strong>人格 / 置信度走势</strong>
      </div>

      <section className="cockpitSection">
        <div className="sectionCaption">人格画像与演化（OCEAN）</div>
        <PersonalityPanel profile={contact.personalityProfile} />
      </section>

      <section className="cockpitSection">
        <div className="sectionCaption">贝叶斯置信度走势</div>
        <BayesianTrendChart signals={contact.bayesianSignals ?? []} />
      </section>
    </section>
  );
}

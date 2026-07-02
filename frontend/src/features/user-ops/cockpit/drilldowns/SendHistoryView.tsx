// 发送历史下钻（Task 5）。SendHistorySection verbatim 迁移 —— 数据流/渲染不变，
// 只在外层套统一的下钻头部（返回按钮 + 标题），与记忆/会话下钻对齐。
import { ArrowLeft } from "lucide-react";
import { SendHistorySection } from "../../legacy";
import styles from "../cockpit.module.css";

export function SendHistoryView({ wxid, onBack }: { wxid: string; onBack: () => void }) {
  return (
    <section className="smartTabPanel">
      <div className={styles.drilldownHead}>
        <button className={styles.backButton} type="button" onClick={onBack}>
          <ArrowLeft size={15} />
          返回
        </button>
        <strong>AI 已发送</strong>
      </div>
      <SendHistorySection wxid={wxid} />
    </section>
  );
}

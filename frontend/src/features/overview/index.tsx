import { Activity, Sparkles } from "lucide-react";
import { MetricCard } from "../../components/ui/MetricCard";
import { EmptyState } from "../../components/ui/EmptyState";
import { useContactStore } from "../../stores/contactStore";
import { useAccountStore } from "../../stores/accountStore";
import { useNavigationStore } from "../../stores/navigationStore";
import styles from "./Overview.module.css";

export default function OverviewFeature() {
  const contacts = useContactStore((s) => s.contacts);
  const managedCount = useContactStore((s) => s.managedCount());
  const normalCount = useContactStore((s) => s.normalCount());
  const onlineCount = useAccountStore((s) => s.onlineCount());
  const setChannel = useNavigationStore((s) => s.setChannel);

  // TODO(operations/user-ops 迁移后接入): pendingTasks / latestEvent 来自各自 store。
  // 在 events store 落地前，Pending Tasks 显示 0、最近事件显示空态。
  const pendingTasks = 0;

  return (
    <section className={styles.overviewGrid}>
      <MetricCard label="Managed Users" value={managedCount} detail="Agent 运营好友" onClick={() => setChannel("userOps")} />
      <MetricCard label="Contact Base" value={contacts.length} detail={`${normalCount} 普通好友`} onClick={() => setChannel("userOps")} />
      <MetricCard label="Account Online" value={onlineCount} detail="可用微信账号" onClick={() => setChannel("overview")} />
      <MetricCard label="Pending Tasks" value={pendingTasks} detail="待执行任务" onClick={() => setChannel("operations")} />

      <section className={styles.widePanel}>
        <div className={styles.panelHead}>
          <div>
            <span>Operating Model</span>
            <h2>AI 私域运营系统</h2>
          </div>
          <Sparkles size={18} />
        </div>
        <div className={styles.principleGrid}>
          <div>
            <strong>独立用户上下文</strong>
            <p>每个 managed 好友拥有运营备注、画像、记忆和跟进节奏，不使用统一批量话术。</p>
          </div>
          <div>
            <strong>双 Agent 架构</strong>
            <p>管理 Agent 负责后台操作，运营 Agent 负责好友、群和朋友圈的长期业务运营。</p>
          </div>
          <div>
            <strong>审计优先</strong>
            <p>回复、任务、策略、工具调用和失败事件进入日志，保证长期运行可复盘。</p>
          </div>
        </div>
      </section>

      <section className={styles.sidePanel}>
        <div className={styles.panelHead}>
          <div>
            <span>Last Event</span>
            <h2>最近事件</h2>
          </div>
          <Activity size={18} />
        </div>
        <EmptyState title="暂无运营事件" />
      </section>
    </section>
  );
}
